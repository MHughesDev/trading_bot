# Phase 1 — The engine core

**Completion: 100% (9 / 9 tasks)**

**Requirement IDs:** SU-ENG-01…09.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §3, §5, §6, §7, §8.4; ADR-0039, ADR-0041, ADR-0042.
**Depends on:** Phase 0.
**Blocks:** Phases 3, 4, 6, 8.
**Migration range:** none.

The pure engine: a compiled plan plus state in, decisions out. Nothing in this phase touches a database, a clock, a venue or a socket. Every test drives it with synthetic events, which is the point — if the engine needs an environment to be tested, it is not pure.

---

### U-1.1 `strategy-engine` crate and the compiler — L
`DONE` 2026-10-01 — new crate `crates/strategy-engine`; `compile.rs` is the only strategy typechecker. Both FEAT-007 §5.2 examples compile, with `ema(close, n)` lowered to the DATA-006 `ema_n` indicator rather than reimplemented and the bracket's ATR distance registering `atr_14`. A three-node cycle is refused naming all three and suggesting `prev(x, 1)`; a `num` in a `bool` port names the node, both types and how to make a condition; a wrongly-typed model input is refused against the contract; `ema(close, 48)` feeding `rolling_mean(·, 20)` warms up for the indicator's lookback plus 20. Every problem is reported in one pass with did-you-mean fixes (`crosed_above` → `crossed_above`, `rsi14` → `rsi_14`). `crates/strategy-engine/tests/phase1_acceptance.rs`.

New crate. `compile(&StrategyGraph, &Registries) -> Result<ExecutionPlan, Vec<Diagnostic>>`: resolve refs, typecheck every port against the operator registry, feature registry and model contracts, topologically order nodes (refuse cycles by naming them), allocate value slots, compute warm-up per (instrument, timeframe), and partition model nodes into dependency layers. Diagnostics carry a code, the node, a fix and an example.

**Files:** `crates/strategy-engine/src/{lib,plan,compile,diagnostic}.rs`.
**Acceptance:** both §5.2 graphs compile; a cycle is refused naming every node in it; a `num` wired into a `bool` port is refused naming both nodes and the expected type; a model input of the wrong type is refused against its contract; warm-up for `ema(close, 48)` feeding `rolling_mean(·, 20)` is the sum the registry implies.
**Traces:** FEAT-007 §5, §10; ADR-0039; ADR-0045.

### U-1.2 Evaluation with absence — M
`DONE` 2026-10-01 — `src/eval.rs`. A sweep over every stateless operator with each wire input missing in turn: no output is a present `num` unless the operator is an absence handler or Kleene logic decides without it. Missing values carry their **root** reason downstream ("vol_forecaster@12 did not answer"), not a chain of "upstream was missing". `if(missing, a, b)` is missing; `coalesce(missing, 3) = 3`; division by zero, `sqrt` of a negative and `exp` overflow are `missing(undefined)`, never `NaN`. `vote` is Kleene-consistent (true if enough are certainly true, false if even every unknown could not reach k).

Evaluate the plan's slots in order: math, logic, comparisons, `if`, `switch`, blending, absence handlers. `Missing` propagates with `upstream(node)`; Kleene logic for `and`/`or`/`not`; division by zero and log of a non-positive are `missing(undefined)`.

**Files:** `crates/strategy-engine/src/eval.rs`.
**Acceptance:** a property test: for any graph of pure operators, a missing input never produces a `num` downstream unless an absence handler intervenes; `if(missing, a, b)` is missing; `coalesce(missing, 3) = 3`; no evaluation path produces `NaN`.
**Traces:** FEAT-007 §5.1, §5.3; QA-5.

### U-1.3 Indicators inside the engine — L
`DONE` 2026-10-01 — `src/history.rs` keeps each input's bars and computes every plan indicator once per bar through `features::runtime::value_at`, unchanged. An `ema(close, 7) > ema(close, 21)` strategy fed **only bars** matches the DATA-006 routine bar for bar over 250 bars, and before warm-up the comparison is `missing(warming_up)` and its rule `Unknown` — not `false`. `history` is a private module and `Event` has no indicator-carrying variant, both asserted, so nothing outside the engine can supply an indicator value — the structural fix for the paper path that never computed one.

The engine owns a bar history per declared (instrument, timeframe), bounded by the plan's warm-up, and computes every feature source through `features::runtime` — the DATA-006 routine, unchanged. Nothing outside the engine supplies an indicator value in any environment. A source still warming up is `missing(warming_up)`.

**Files:** `crates/strategy-engine/src/sources.rs`.
**Acceptance:** an `ema(close, 7) > ema(close, 21)` graph fed only bars produces the same values as `features::runtime::value_at` over the same bars (equality, every bar); before warm-up completes the comparison is `missing(warming_up)`, not `false`; there is no public API through which a caller can write an indicator value.
**Traces:** FEAT-007 §1 defect 2, §3; ADR-0039; ADR-0029.

### U-1.4 Temporal operators — M
`DONE` 2026-10-01 — `src/temporal.rs`: `prev`, `change`, `crossed_above/below`, `rising/falling`, `rolling_*` and `ema` over node outputs, `bars_since`, `count_true`, `latch`, `hysteresis`, `hold_last`, `cooldown`, `zscore`, `pct_rank`. A crossing fires exactly once; a missing value poisons a window until it rolls out; a tick resumed through two model layers advances `count_true` once (have 1 of 3). **Mutation-checked:** letting operator nodes re-evaluate on resume fails exactly that test.

Per-node state for `prev`, `change`, `crossed_above/below`, `rising/falling`, `rolling_*` and `ema` over any node's output, `bars_since`, `count_true`, `latch`, `hysteresis`, `hold_last`, `cooldown`. State advances once per decision tick, never on a resume.

**Files:** `crates/strategy-engine/src/temporal.rs`.
**Acceptance:** `crossed_above(a, b)` fires exactly once per crossing over a scripted series; a missing value inside a rolling window makes the window missing until it has rolled out; `hold_last` keeps the last present value and reports its age; a model resume does not advance temporal state (asserted by resuming twice).
**Traces:** FEAT-007 §5.3; FEAT-004 §3.3 (prev-value registers).

### U-1.5 State from the portfolio view — S
`DONE` 2026-10-01 — `src/portfolio.rs`, `engine.rs::state_value`. All thirteen `state.*` fields are read from the `PortfolioView` the environment passes with each event (asserted present when the view has them); a source scan asserts the engine and plan hold no position of their own.

`state.*` sources read a `PortfolioView` the environment passes with each event — position, position fraction, entry price, bars in position, unrealised return, realised P&L today, equity, cash, open orders by tag, last fill price, last exit reason, mark. The engine keeps no guess of its own.

**Files:** `crates/strategy-engine/src/state.rs`.
**Acceptance:** every `state.*` field is read from the view (a test supplies a view and asserts each); the engine has no field holding a position (a source-scan guard).
**Traces:** FEAT-007 §5.4; §1 defect 9.

### U-1.6 Model calls by request and resume — L
`DONE` 2026-10-01 — `src/models.rs`, `engine.rs`. A classifier → sizing-model chain with an indicator through `if` produces two request layers in order (`dir`, then `size`); the classifier's `probs` drive `prob(p, "up") > 0.6` unreshaped and the second model's fraction sets the target size. An upstream abstain means the downstream model is **never called** (counted: zero calls over 120 bars); a row whose indicators are still warming up is never sent — **mutation-checked**: zero-filling it fails `u_1_6_model_inputs_are_never_zero_filled`. Answers in either arrival order give identical decisions. Responses are checked against the contract: a missing or mistyped field, or non-monotone quantiles, become an abstain naming the field; an unanswered request becomes a timeout rather than waiting for ever.

`on_event` evaluates to the first unresolved model layer and returns `Step::NeedModels(requests)`, each request carrying the model ref, pinned version, the assembled typed inputs and the decision instant; `resume(responses)` continues. Model outputs become typed `record`s; adapters `argmax`, `prob`, `quantile`, `mean`, `prob_above`, `cosine`, field access. An abstain or timeout is the corresponding `missing`.

**Files:** `crates/strategy-engine/src/models.rs`.
**Acceptance:** a graph chaining model A into model B and an indicator through `if` produces two request layers in order; resuming with A's abstain makes B's input missing and B is not requested; a classifier's `probs` drives `prob(p, "up") > 0.6` without any reshaping; the same responses in a different arrival order give identical decisions.
**Traces:** FEAT-007 §8.4; ADR-0043 decision 6.

### U-1.7 Rules to intents — M
`DONE` 2026-10-01 — `src/decide.rs`. A target sized by a model output that timed out yields **no order** and the rule's outcome is `Held { reasons: [model_timeout frac@1, 100 ms] }`. First match wins within a group and a second group acts on the same tick. A selection graph cannot reach `decide`: the domain type makes it unrepresentable (Phase 0), and an engine for a selection plan is refused until Phase 6.

Evaluate `decide` rules in order on each decision tick; the first whose `when` is `true` contributes its intent (rules may be grouped so an entry rule and a protect rule can both fire). A `when` that is `missing` does not match. An intent whose numeric fields are missing becomes `hold` with the missing reasons attached.

**Files:** `crates/strategy-engine/src/decide.rs`.
**Acceptance:** a rule whose size depends on a missing model output yields `hold` carrying `model_timeout`; first-match ordering is asserted; a selection graph reaching `decide` is impossible (compile-time refusal tested).
**Traces:** FEAT-007 §5.1, §6; ADR-0041; QA-5.

### U-1.8 The order planner — L
`DONE` 2026-10-01 — `src/planner.rs`, 16 tests. Re-stating an unchanged target emits nothing; a target half-filled by a working order emits nothing more; a contradicted working entry is cancelled and the remainder placed; a negative target shorts and a target can flip; rounding never turns a reduce into a reversal; close half keeps protection and close all withdraws it; a bracket places its stop and target **only when the entry fills**, anchored at the fill price (2% under 101 → 98.98) and OCO-grouped; when one protective leg fills the others are cancelled (OCO emulated for venues without it); `protect` moves a stop to breakeven and refuses while flat; a limit on a market-only venue is refused by name, never downgraded; prices round in the never-more-aggressive direction; risk sizing sizes by the stop distance; every row of FEAT-007 §6's table is planned.

Intents + `PortfolioView` + instrument metadata → order actions (`Submit`, `Cancel`, `Replace`). Target diffing against position *plus working orders*; lot and tick rounding; minimum size; cancelling a working entry a new target contradicts; bracket and protect as OCO groups tracked by tag (emulated when the venue lacks native OCO); close/add/reduce; refusing an intent the venue cannot express, by name.

**Files:** `crates/strategy-engine/src/planner.rs`.
**Acceptance:** re-evaluating an unchanged `target` emits no orders (idempotence); a target half-filled by a working order emits only the remainder; rounding never rounds a reduce-only order into a reversal; a `bracket` emits its protective pair only after the entry fill event; every case in FEAT-007 §6's table has a test.
**Traces:** FEAT-007 §6; ADR-0041.

### U-1.9 Decision trace and determinism — M
`DONE` 2026-10-01 — `src/trace.rs`. 150 ticks of a two-model strategy run twice serialise to byte-identical traces. The acceptance asked for a seeded-hasher permutation test; a stronger structural guard replaces it: **no hash-ordered collection exists anywhere in the engine** (source scan over every module), so no output can depend on iteration order. Also: a Cargo.lock walk proves `strategy-engine` reaches none of `execution`, `risk`, `storage`, `model-registry`, `tokio`, `sqlx`, `reqwest`, with a positive control so a blind parser cannot pass it vacuously.

Every step returns a `DecisionTrace`: instant, input-value digest, model requests and responses (version, fold), intents, planned actions, missing reasons. Output ordering never depends on hash iteration.

**Files:** `crates/strategy-engine/src/trace.rs`.
**Acceptance:** running the same plan over the same events twice yields byte-identical serialised traces; shuffling the insertion order of every internal map does not change them (a test builds the engine under a seeded hasher permutation).
**Traces:** FEAT-007 §11; ADR-0047; QA-7.
