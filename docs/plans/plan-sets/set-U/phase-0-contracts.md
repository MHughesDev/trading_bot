# Phase 0 — The contracts

**Completion: 0% (0 / 5 tasks)**

**Requirement IDs:** SU-CON-01…05.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §5, §6, §8.1; ADR-0040, ADR-0041, ADR-0043.
**Depends on:** nothing.
**Blocks:** every other phase.
**Migration range:** none — `domain` types only.

Everything else in this set is written against these types. They are data only — no behaviour, no I/O — and they are the part that has to be right first, because the engine, the language front-end, the model registry and the environments all speak them.

---

### U-0.1 Values, types and absence — M
`NOT STARTED`

`domain::strategy_graph::value`: `ValueType` (`num`, `bool`, `cat<classes>`, `probs<classes>`, `dist<levels>`, `vec<n>`, `text`, `record{…}`) and `Value`, where any value may be `Missing(MissingReason)` — `warming_up`, `no_data`, `model_abstained`, `model_timeout`, `model_unavailable`, `undefined`, `upstream(node)`. No `NaN` is representable as a `num`; constructing one yields `missing(undefined)`. Kleene `and`/`or`/`not` are defined here, once.

**Files:** `crates/domain/src/strategy_graph/value.rs`.
**Acceptance:** a `num` cannot hold `NaN` or an infinity (a property test over arbitrary `f64`); `false and missing = false`, `true or missing = true`, `not missing = missing` — the full truth table is asserted; every `MissingReason` serialises and round-trips; `Value` serialisation is stable (golden bytes).
**Traces:** FEAT-007 §5.1; ADR-0040 decision 2; QA-5.

### U-0.2 The canonical graph — M
`NOT STARTED`

`StrategyGraph` (`format: tbot.strategy/3`): `kind` (`trading` | `selection`), name, asset class, decision timeframe, typed params and constraints (reusing v1.2 `ParamSpec`), declared extra inputs (other instruments and timeframes), `nodes` (`id`, `op`, `args` binding ports to node refs, `node.field` refs, params or literals), `decide` (ordered rules + `otherwise`) **or** `select` (score, where, top_n, order, attributes), `triggers`, and tighten-only `risk`. Canonical JSON and its content hash are the revision.

**Files:** `crates/domain/src/strategy_graph/mod.rs`, `graph.rs`, `revision.rs`.
**Acceptance:** a trading graph and a selection graph from FEAT-007 §5.2 deserialise, re-serialise byte-identically through canonical JSON, and hash stably; a graph with both `decide` and `select`, or neither, is refused at deserialisation by name; node ids are unique (refused otherwise).
**Traces:** FEAT-007 §5.2; ADR-0040; ADR-0010.

### U-0.3 Trade intents, selections and deployments — M
`NOT STARTED`

`TradeIntent` (`hold`, `target`, `order`, `bracket`, `protect`, `close`, `add`, `reduce`, `cancel`), `SizeSpec` (`units`, `notional`, `equity_fraction`, `risk`, `vol_target`), `Distance` (`pct`, `abs`, `atr(k, n)`, `sigma(k)` from `entry` | `mark` | `high_water`), the full order vocabulary (market, limit, stop, stop-limit, trailing stop; TIF; reduce-only; post-only; tag). Every numeric field is a `ValueRef`. `Selection` (ranked `{instrument, score, attributes}` + `as_of`). `DeploymentTarget` (`Instrument` | `Selection { selection, top_n, rebalance, on_removal, weighting }`).

**Files:** `crates/domain/src/strategy_graph/intent.rs`, `selection.rs`, `deployment.rs`.
**Acceptance:** every intent kind in FEAT-007 §6 round-trips; a `bracket` without a `stop` is refused; `close.fraction` outside (0, 1] is refused; an `order` of type `limit` without a price is refused by name.
**Traces:** FEAT-007 §4, §6; ADR-0041.

### U-0.4 Model IO contracts — M
`NOT STARTED`

`IoContract`: typed named inputs (`scalar`, `row(feature_set)`, `window(feature_set, lookback, timeframe)`, `cross_section`, `text(template)`) and typed named outputs, horizon, PIT metadata (walk-forward, embargo), cost class, timeout. `derived_contract(ModelKind, bundle_header)` for every existing kind: Forecaster, TradeDecision, RiskSizing, SignalRanker, Embedding, ExternalLlmAdapter. New kinds `classifier`, `regressor`, `transform`.

**Files:** `crates/domain/src/model_def/contract.rs`, `kinds.rs`.
**Acceptance:** a derived contract exists for every `ModelKind` (exhaustive match, so a new kind cannot be added without one); a forecaster's derived contract exposes `probs`, `num` and `dist` outputs — not a single direction string; a contract with two outputs of the same name is refused.
**Traces:** FEAT-007 §8.1; ADR-0043 decisions 1, 2.

### U-0.5 The operator registry — M
`NOT STARTED`

One table of operators: name, input ports and their types (with generics for `cat`/`probs` labels and `dist` levels), output type, warm-up, statefulness, whether it is cross-sectional, and a one-line *use when*. Sources, math and logic, branching, temporal, models and adapters, blending, cross-sectional, absence — FEAT-007 §5.3. The typechecker, editor palette, SLv3 grammar and agent catalogue are generated from it (later phases); this task is the table and its self-consistency tests.

**Files:** `crates/domain/src/strategy_graph/ops.rs`.
**Acceptance:** every operator in FEAT-007 §5.3 is in the table; no two share a name; every stateful operator declares a warm-up; every cross-sectional operator is marked and refused in a trading graph by a test over the table; the table serialises for the catalogue.
**Traces:** FEAT-007 §5.3; ADR-0040; QA-3.
