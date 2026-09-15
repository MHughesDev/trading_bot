# Phase 5 — The language

**Completion: 0% (0 / 15 tasks)**

**Requirement IDs:** SA-01…SA-16.
**Spec:** [AGENT-011](../../../specs/AGENT-011-strategy-authoring-for-models.md), which
revises [FEAT-004 §3](../../../specs/FEAT-004-strategy-representation-v2.md) and
[DATA-006 §5](../../../specs/DATA-006-unified-feature-engine.md). ADR-0010 (canonical JSON),
ADR-0029 (one engine).
**Depends on:** Phase 1 track A only. Independent of the runtime work.
**Blocks:** Phase 6's strategy editor (S-6.21); Phase 8's `signal_identity` workflow.
**Migration range:** 0082–0084.
**Parallel with:** Phases 3, 4, 6, 7.

Everything else in Set S makes the agent reason better and waste less. This phase is the
only one that lets it *say more*. Today a strategy rule can use two feature families
(`ema_N`, `rsi_N`) and a comparison grammar with no functions — while DATA-006 specifies
forty operator families and FEAT-004 specifies a typed language with exits, sizing, state
predicates and structural moves, both unbuilt. A previous session recorded this as the actual
blocker on the train→strategy→backtest chain.

---

## Two hard constraints, read before writing any code

**AT-15 bans a second EMA/RSI implementation.** `crates/invariants/tests/static_invariants.rs`
walks every `.rs` under `crates/` and `apps/` and fails the build on a parallel indicator
implementation. The F1 engine therefore **extends** `crates/features/src/{ema,rsi}.rs` — it
does not sit beside them. A genuine exception needs an allowlist entry *and* an ADR row;
the test's own header says never to widen the allowlist without recording why.

**`builders-purity` bans I/O in `crates/features`.** A CI job runs `cargo tree -p features
--edges normal` and fails on `sqlx`, `clickhouse`, `redis`, `async-nats`, `event-bus` or
`storage`. The engine stays a pure function of data passed as arguments. This is invariant 4
in `AGENT.md` ("same builder code live and in replay") and it is not negotiable.

---

## The feature engine

### S-5.1 F1 families, inside the existing engine — L
`NOT STARTED`

Arithmetic and math; `ret lag diff`; `rolling_mean/std/var/min/max/sum/median/quantile/skew/
kurt`, `ewma`, `zscore`, `pct_rank`; `sma ema rsi atr macd bb donchian keltner roc`;
calendar (`hour_sin/cos`, `dow_sin/cos`, `session`). This is ~80% of expressible rule ideas,
ATR-unit exits, z-score mean reversion and session filters.

**Files:** `crates/features/src/` (extending `ema.rs`, `rsi.rs`).
**Acceptance:** the golden parity test DATA-006 §5 requires — old and new values identical
for each legacy alias over 3 instruments × 3 timeframes — passes before any legacy code is
deleted; `static_invariants` and `builders-purity` both green.
**Traces:** AGENT-011 §2 F1; §8.1.

### S-5.2 The registry and `list_features` — M
`NOT STARTED`

Each family registered with parameters and ranges, output dtype, warm-up bars, PIT lag, cost
class (`cheap | rolling | model`) and a one-line "use when". `list_features(family?,
cost_class?, query?)` serves it — this is the tool that makes the language discoverable
rather than memorised.

**Files:** `crates/features/src/registry.rs`, `migrations/0082_feature_registry.sql`,
`crates/mcp-server/src/tools/strategy.rs`.
**Acceptance:** every feature the backtest accepts is served by `list_features`, and every
feature listed is accepted by the backtest, the live runtime **and** the Phase 3 analysis
templates. One engine, proven by test.
**Traces:** AGENT-011 §2, P2; §8.1.

### S-5.3 The validator teaches — M
`NOT STARTED`

`ValidationError` gains `fix` and `see`. A table in `crates/strategy-validator/src/teach.rs`
maps each error code to a concrete edit: `unknown_feature` → "did you mean `rsi(close, 14)`?
`list_features(query="rsi")`"; `distance_without_unit` → "write `1.5 atr` / `1.5 %` /
`1.5 sigma` from entry"; `when_names_expression` → "bind the expression as an input";
`warmup_exceeds_window` → "start earlier or shorten the longest lookback". Three rejections
on the same `path` additionally return a worked example — the cheapest understanding gain on
a small tier is seeing a correct instance, not another rule.

**Acceptance:** the six error cases in AGENT-011 §5 return the listed `fix`; a third
identical rejection returns an example; `rejects_loosening` still refuses a widened risk
envelope.
**Traces:** AGENT-011 §5, P4; §8.4.

### S-5.4 `preview_signals` — look before you spend — L
`NOT STARTED`

A dry run over stored bars through the feature engine and the exit policy: entries per month,
sample events with exit reasons, warm-up status, and `never_fires_because` when it does not.
**No trial, no ledger row.** This is the tool that changes behaviour most — step 1 of the
research protocol becomes write → preview → adjust → create, and a strategy that never fires
or fires every bar is caught before a counted trial.

**It obeys the research cutoff and the sealed holdout.** This is where "no trial" could
quietly become "no rules": a preview that could read holdout bars lets an agent tune against
the holdout for free, one call at a time, and the gate funnel would never see it — the
cheapest route around the platform's entire honesty stack, through the tool most designed to
be called often. A window overlapping the holdout is **refused, naming the partition**, never
silently truncated, because a silent truncation would disagree with the later backtest and
teach the model to distrust the tool.

**Acceptance:** it writes no trial row; its `sample_events` match the fills of a subsequent
backtest on the same window for the first ten events; a window overlapping the sealed holdout
is refused naming the partition. Adversarial: the refusal is asserted.
**Traces:** AGENT-011 §4, P3; §8.5, §8.5a.

---

## SLv2

### S-5.5 The parser — L
`NOT STARTED`

New crate `crates/strategy-lang`. FEAT-004 §3.1's grammar with the AGENT-011 §3 amendments,
all of which point one way — a small model writing it correctly: every clause on its own
line keywords-first; `inputs` mandatory for anything used twice and for any expression over
120 characters; units explicit on every distance; `crossed_above/below`, `rising/falling`,
`between`, `abs/min/max/clip` compiler-provided; state predicates; typed `params` with
ranges; `order` clauses defaulting to market. No user-defined functions, no loops, no
multi-instrument in v2.0 — everything above is grammar-constrainable, and anything
Turing-ish is `run_python`.

**Acceptance:** a bare number in a distance is a compile error naming the three legal units;
an inline expression over 120 characters is refused with "bind it as an input".
**Traces:** AGENT-011 §3, P1, P7.

### S-5.6 The compiler — M
`NOT STARTED`

AST → the canonical `StrategyDefinition` (`crates/domain/src/strategy_def/mod.rs:64`). JSON
remains the storage and runtime form (ADR-0010); it stops being what a model is asked to
produce.

**Acceptance:** a compiled definition is byte-identical to the equivalent hand-written JSON
for the three skeletons.
**Traces:** AGENT-011 §3, P1.

### S-5.7 The decompiler and the round trip — M
`NOT STARTED`

JSON → canonical text, so every stored strategy has a readable form and the v1 expression
grammar remains a subset the decompiler emits when nothing newer is used.

**Acceptance:** SLv2 → JSON → text → JSON yields an identical definition (round-trip test).
**Traces:** AGENT-011 §3; §8.3.

### S-5.8 The authoring tools — M
`NOT STARTED`

`compile_strategy` (never persists), `explain_strategy` (reads a strategy back in words),
`create_strategy(text)` recording `parent_version`, and the upgraded `validate_strategy`
accepting text.

**Acceptance:** `explain_strategy` names every input, entry, exit, size rule and the warm-up
bars; `create_strategy` records lineage for `family_test`.
**Traces:** AGENT-011 §4.

### S-5.9 A grammar for the small tiers — M
`NOT STARTED`

A clause-level JSON-Schema grammar (one clause per fill) rather than the EBNF, so a
`select_fill` tier writes SLv2 under constrained decoding and the compiler assembles the
text.

**Acceptance:** on `qwen2.5-7b`, a research goal produces a compiled SLv2 strategy in ≤ 6
fills with no `unknown_feature` error, on the eval dataset.
**Traces:** AGENT-011 §3, P7; §8.7.

### S-5.10 Moves, diffs, lineage — M
`NOT STARTED`

`diff_strategies` classifies an AST diff into the FEAT-004 §6 move kinds and says what
changed **in words**, with `is_single_change`; `apply_move` produces one new validated
version per move. This makes the research protocol's "change one thing at a time"
checkable rather than exhorted.

**Acceptance:** every move kind round-trips from `apply_move` through `diff_strategies`;
`is_single_change` is false for a two-move version.
**Traces:** AGENT-011 §4, P5; §8.6.

### S-5.11 `suggest_moves` — S
`NOT STARTED`

Maps `get_diagnostics` findings to candidate moves with a reason ("loses in high-vol months"
→ "add_regime_gate on `rv(20) pct_rank < 0.8`"). Deterministic rules first; model ranking
only on capable tiers. Motivated moves only — exploration belongs in the Experiment step.

**Acceptance:** every suggestion cites the diagnostic that motivated it.
**Traces:** AGENT-011 §4, §10.

### S-5.12 Skeletons — S
`NOT STARTED`

`strategy_skeleton(hypothesis_family)` for trend, mean reversion, breakout, carry,
vol target and model signal, so a first version is never a syntax lesson.

**Acceptance:** the trend, mean-reversion and vol-target skeletons compile, validate and
preview on BTC-USD 1h with non-zero entries and no warm-up violation.
**Traces:** AGENT-011 §4; §8.2.

### S-5.13 Rewrite the guide around the language — M
`NOT STARTED`

Replace the "Strategy definition format", "Expression grammar" and "Features available"
sections with one page of grammar, a worked example per hypothesis family, the units table,
the moves table, and per-family feature notes generated from the registry. JSON moves to an
appendix for the MCP client.

**Acceptance:** each SLv2 section is ≤ 4 KB (so a 7B fetches one at a time) and every `see`
anchor from S-5.3 resolves.
**Traces:** AGENT-011 §6; §8.8. AGENT-005 §3.3.

### S-5.14 F2 families — L
`NOT STARTED`

Range volatility (`parkinson … yang_zhang`), realised measures (`rv bv rq jump_share`),
`hurst variance_ratio autocorr fracdiff`, `efficiency_ratio`, `lookback_ensemble` — regime
gates, vol targeting, persistence filters, and the same code the Phase 3 analysis templates
read.

**Acceptance:** the analysis templates and the strategy engine compute identical values for
the same family on the same window.
**Traces:** AGENT-011 §2 F2, P2.

### S-5.15 Intent survives, and the preview says which clause blocked — M
`NOT STARTED`

**Comments are part of the language.** A strategy is read by a reviewer, by the model on its
next turn, and by whoever opens the dossier in six months, and none of them can recover why a
threshold is 0.6 from the number. The parser attaches `# …` to the following clause, the
compiler stores it on the canonical node, the decompiler emits it back — so intent survives
compile → JSON → decompile, which is exactly where it is lost today; `apply_move` carries it
forward and appends the move's reason.

**The preview names the blocker.** `never_fires_because` says something is wrong, not what, so
the next move is a guess costing a round trip. Evaluate each clause independently over the
window and return a blocking profile: per clause, the share of bars it was true, and for the
conjunction the share where it was the *last* clause to fail. "`rsi(close,14) < 30` was true
on 0.4% of bars and was the sole blocker on 91% of the rest" turns "it doesn't fire" into
"this threshold is the problem" in one step, and it is what `suggest_moves` reads so a
suggestion cites a measurement rather than a heuristic.

**Files:** `crates/strategy-lang/`, `crates/domain/src/strategy_def/`, preview handler.
**Acceptance:** a comment survives the full round trip and appears in `explain_strategy`; the
blocking profile identifies the sole blocker on a fixture built to have one.
**Traces:** AGENT-011 §3, §4; §8.3.

---

## Exit criteria

- One feature engine serves backtest, live runtime, analysis templates and the training
  frame, with no second EMA/RSI implementation and no I/O in `crates/features`.
- A strategy can be written as text, previewed without spending a trial, and explained back
  in words.
- SLv2 round-trips through the canonical JSON.
- A 7B writes a valid strategy under a grammar in ≤ 6 fills.
- The validator returns a concrete fix for every error class in AGENT-011 §5.
