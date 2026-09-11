# BS-007 / 08 — Strategy representation: two layers

**Spec status:** spec-ready draft · **Req prefix:** ST- · **Formal spec:** FEAT-004 (and
DATA-004 v2 for the AST format) · **ADR:** ADR-0026 (two-layer strategies) · **Plan set:**
M (Layer 1) / O (Layer 2) · **Decisions:** D-08 · **Gaps fixed:** G-04, G-05 (with 09)

---

## 1. Why change it

The current grammar allows:
- one comparison;
- no functions;
- no lookback;
- no state;
- one action (`place_order`).

The JSON node graph was built for drag-and-drop, not for an agent that writes code
(01 §3). Adding node kinds one at a time won't fix the expression model. Research needs
unlimited expressiveness; deployment needs safety, speed and parity. So there are two
layers.

| | Layer 1: research strategy | Layer 2: Strategy Language v2 (SLv2) |
|---|---|---|
| Form | Python function in the workspace: PIT data → target positions | Text DSL → AST (canonical JSON) → bytecode VM |
| Written by | The agent (or a user in a notebook) | The agent, a user, or the UI editor (over the AST) |
| Runs in | Research-runner sandbox (05 §3) | Backtest engine and live `StrategyInstance` |
| Can use | Anything computable from PIT data: models, text features, cross-asset | Feature engine functions, typed model calls, state, exits, sizing, orders |
| Leakage control | Mechanical truncation test (§3.3) | Close-stamped features, engine-enforced warm-up |
| Evaluated by | Set J, as `position_series` runs | Set J, as `definition` runs |
| Deployable | **No.** Must be translated to Layer 2 and reconciled (§7) | Yes, after the funnel and human approval |

## 2. Shared rules

- Both layers are counted identically (INV-1). Both carry a strategy version hash.
  Deployed versions are immutable, and a change restarts the funnel (11 §4, rule 5c).
- Tunable parameters are declared with ranges. Values come from sweeps (P1). A strategy
  with hard-coded "magic numbers" inside a declared-param strategy is linted.

## 3. Layer 1: research strategies (position series)

### 3.1 API

```python
# strategies/vol_breakout.py
from tbot.strategy import research_strategy, Param, target_vol_scale

@research_strategy(
    instruments=["BTC-USD"], timeframe="1h",
    params={"lookback": Param.int(10, 100), "k": Param.float(0.5, 3.0)},
)
def positions(data, p):
    rv  = data.feature("rolling_std(logret(close), 20)")
    rng = data.feature(f"rolling_max(high, {p.lookback}) - rolling_min(low, {p.lookback})")
    signal = (data.close > data.close.shift(1) * (1 + p.k * rv)) & (rng.pct_rank(250) < 0.2)
    return signal.astype(float) * target_vol_scale(rv, 0.4)   # target position in [-1, 1]
```

- **Input:** a `data` object over PIT frames:
  - bars, features (07), prediction series (10 §5), text features, other instruments
    declared in `instruments`;
  - everything clipped at the project cutoff and each bar's `available_time`.
- **Output:** a target-position series indexed by decision time, as a fraction of equity
  in [−1, 1] (or units, if declared).
- **Purity:** no I/O, no clock, no randomness without a seeded `data.rng`. The runner
  enforces this: no network, and a read-only filesystem except scratch.

### 3.2 Execution

`tbot bt run strategies/vol_breakout.py --experiment exp_…` submits a `position_series`
job (05):
- the runner snapshots the code (hash in the manifest) and evaluates the function;
- the **backtest service executes the positions in the engine** under an explicit
  execution policy: decide at bar close *t*, fill at *t+1* open by default (09 §3), with
  costs from the instrument's cost model (09 §5).

**The agent never computes its own P&L.**

### 3.3 Truncation test (mechanical look-ahead check)

- The service re-evaluates the function on data truncated at K random points *k*
  (default K=8, stratified). Positions at every time ≤ *k* must match the full-data run
  exactly.
- Any difference is look-ahead. Gate 0 fails, and the report names the first differing
  timestamp.
- This catches full-sample normalisation, `bfill`, centred windows, and future-indexed
  joins. It works because the strategy is a function.

### 3.4 Sweeps

The platform sampler (FEAT-003 Random/TPE) calls the function with sampled params inside
the runner. The agent declares ranges and never picks values (P1). It never sees an
argmax (INV-2).

## 4. Layer 2: Strategy Language v2

### 4.1 Example

```text
strategy vol_breakout_v3 on $instrument timeframe 1h
params  lookback: int[10..100] = 40,  k: float[0.5..3.0] = 1.2,  tv: float[0.1..0.8] = 0.4
inputs  rv   = rolling_std(logret(close), 20)
        rng  = rolling_max(high, lookback) - rolling_min(low, lookback)
        f    = model("vol_forecaster@production").sigma      # typed model output
entry long  when close > close[1] * (1 + k * rv) and pct_rank(rng, 250) < 0.2 and flat
size        vol_target(tv, f)  clamp 0..1
exit        stop   2.0 * atr(14) from entry
            target 4.0 * atr(14) from entry
            time   48 bars
risk        max_position 1.0   max_orders_per_min 6
```

### 4.2 Language

- **Expressions** (shared with feature expressions, 07 §3):
  - the full feature library;
  - lookback indexing `x[k]`;
  - arithmetic, comparison and boolean operators;
  - math (`abs, min, max, log, exp, sqrt, sign, clip`);
  - `crossed_above/below`, `pct_rank`, `lookback_ensemble`, `regime()`;
  - `prediction(series)` (research only), `model(alias).field` (typed `InferenceOutput`
    fields, with an abstain policy);
  - `combine(...)` (equal weights by default; 11 T6).
- **State:** `position`, `flat`, `long`, `short`, `entry_price`, `bars_in_trade`,
  `unrealized_return`, `equity`, `last_exit_reason`.
- **Entries:** `entry long|short when <expr>`, with optional hysteresis and cooldown.
- **Exits:** stop, target, trailing and time, in %, ATR or σ units. They compile to
  bracket or OCO orders in the engine (09 §2).
- **Sizing:** fixed, fraction of equity, risk per trade, `vol_target(σ*, forecast)`, Kelly
  fraction (capped), model-driven, with `clamp`.
- **Orders:** market, limit (offset), stop, stop-limit; time-in-force.
- **Risk:** max position, max orders per minute, max daily loss. These are tighten-only
  relative to the platform risk gate.
- **Timeframes:** multi-timeframe inputs.
- **Multi-instrument** references (pairs, portfolios): a separate decision to lift
  OQ-040. The grammar reserves `universe` and `rank` sections.

### 4.3 Pipeline

Text → parser → **AST (canonical JSON; DATA-004 v2)** → type checker and validator →
compiler → existing bytecode VM (`crates/strategy-runtime/src/bytecode.rs`) and slot
bindings.

- Validation errors teach: they give line, column, rule and fix, like today's validator.
- The AST is the storage format, the UI's edit target, and the diff unit for moves
  (research/R1 §2.2).

## 5. Structural constructs (replacing "node kinds")

Most "new node kinds" become **functions** in the language. The AST keeps a small set of
structural constructs: `strategy`, `params`, `inputs`, `entry`, `exit`, `size`, `risk`,
`model_binding`, and (later) `universe` / `rank`. The 14 existing node kinds (01 §3) map
onto these or onto functions in the v1 → v2 translator.

## 6. v1 → v2 translation

- Every saved v1.x JSON definition translates mechanically into a v2 AST. The translator
  is tested on every definition in the database.
- v1 frontend exits that were dropped (`toDefinition.ts:288`) become v2 `exit` clauses the
  next time the strategy is saved in the new editor.
- Slugs keep working. Versions become immutable hashes, which fixes slug overwrite
  (G-06).

## 7. Research → deployment reconciliation

Promoting a Layer 1 strategy requires a Layer 2 translation. The platform runs both on
the same data and compares the position series (Set J `reconcile` module). Promotion is
blocked unless:
- position agreement ≥ 99% of decision times (configurable);
- the P&L tracking difference is within tolerance;
- every difference is explained by a declared SLv2 approximation.

## 8. UI

The strategy builder becomes an editor over the AST: text view plus structured view,
both editing the same AST. The palette lists feature engine functions, not a fixed set.
This is a byproduct of the agent-first design (D-05).

## 9. Requirements

| ID | Requirement | Pri |
|---|---|---|
| ST-01 | Layer 1 MUST support the §3.1 API, with PIT-clipped data access and a purity-enforcing runner | MUST |
| ST-02 | `position_series` runs MUST be executed by the backtest service under an explicit execution policy and cost model; client-computed P&L MUST NOT be accepted | MUST |
| ST-03 | The truncation test MUST run on every `position_series` Experiment at Gate 0, failing on any pre-truncation difference | MUST |
| ST-04 | Layer 1 sweeps MUST use the platform sampler inside the runner (P1, INV-2) | MUST |
| ST-05 | SLv2 MUST implement §4.2 (expressions, state, entries, exits, sizing, orders, risk, typed model calls, multi-timeframe) | MUST |
| ST-06 | SLv2 MUST compile through AST → validator → bytecode, with teachable errors | MUST |
| ST-07 | The AST MUST be the canonical storage and UI edit format (DATA-004 v2) | MUST |
| ST-08 | Every v1.x definition MUST translate to v2; the translator MUST be tested on all stored definitions | MUST |
| ST-09 | Strategy versions MUST be immutable content hashes; deployed versions MUST NOT change | MUST |
| ST-10 | Layer 1 → Layer 2 promotion MUST pass §7 reconciliation | MUST |
| ST-11 | `combine` MUST default to equal weights; estimated weights need separate validation | MUST |
| ST-12 | Multi-instrument constructs MAY be added after the OQ-040 decision | MAY |

## 10. Acceptance criteria

1. The §3.1 strategy runs as a `position_series` Experiment through Gate 2. A variant
   using `bfill` fails the truncation test, which names the first differing timestamp.
2. The §4.1 SLv2 strategy compiles, backtests with a stop, target, time exit and
   vol-target sizing, and produces the same trades in a replay-determinism test.
3. All stored v1 definitions translate and re-validate.
4. A Layer 1 strategy and its SLv2 translation reconcile above threshold; a deliberately
   different translation is blocked.

## 11. Open questions

1. Multi-instrument SLv2 (lift OQ-040): which phase?
2. Reconciliation thresholds per timeframe.
3. Should Layer 1 allow short-selling or leverage beyond [−1, 1] by default? Recommended:
   no, declared explicitly.
