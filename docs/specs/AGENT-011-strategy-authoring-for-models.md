# AGENT-011: Strategy Authoring for Models — the surface a model writes strategies through, and the feedback that teaches it

> **Status:** **Partially implemented** — Set S phase 5. Re-checked 2026-09-21.
>
> `crates/features` computes ~50 families and the simulator resolves them; `list_features` and `preview_signals` are live. **`crates/strategy-lang` does not exist**, so the language itself — SLv2 text, the parser, the validator — is not built. This is the platform's capability ceiling.
>
> **The live count is in [`docs/plans/plan-sets/set-S/phase-5-strategy-language.md`](../plans/plan-sets/set-S/phase-5-strategy-language.md), and the programme total is in [`docs/plans/PLAN.md`](../plans/PLAN.md).** It is deliberately not restated here: a number written into a spec is a number nobody recomputes, which is how this line came to say "ready to build" about code that was already serving traffic. `crates/invariants/tests/the_plan_counts_what_the_plans_say.rs` holds those two files honest; nothing holds a hand-copied number here.
**Date:** 2026-09-14
**Author:** Claude (Fable 5.1), from `crates/mcp-server/docs/authoring_guide.md`, `crates/strategy-validator`, `crates/features`, `crates/domain/src/strategy_def`, DATA-004, DATA-006, FEAT-004, ADR-0010, ADR-0029
**Revises:** FEAT-004 §3 (SLv2 becomes the model-facing language, with the amendments in §3), DATA-006 §5 (the operator catalogue is phased by what strategies need, §2), the authoring guide (§6)
**Depends on:** AGENT-006 (tool contract; `strategy.*` namespace), AGENT-008 (the verify rung a strategy passes before a trial), AGENT-007 (move memory)

---

## 0. Why this spec exists

Everything in AGENT-005 through AGENT-010 makes the agent reason better and waste less. None of it lets the agent *say* more. Today a strategy is a JSON definition whose rules use a frozen expression grammar with two primitives — `feature('name')` and `bar('field')` — no functions (`authoring_guide.md:84-105`: "no abs/min/max, no lookback indexing, no cross() helper"), and the backtest accepts exactly two feature families: `ema_N` and `rsi_N` (`authoring_guide.md:107-116`; `crates/features/src/{ema,rsi}.rs` are the whole engine). A previous session recorded this as the actual blocker on the train→strategy→backtest chain (memory: `agent-long-task-findings`).

Meanwhile DATA-006 §5 specifies forty operator families (rolling stats, range volatility, realised measures, Hurst, liquidity, order flow, calendar, regime, prediction) and FEAT-004 §3 specifies a typed text language (SLv2) with entries, sizing, exits, risk, `crossed_above`, state predicates, typed parameters and structural *moves*. Both are "Proposed, not implemented". The expressive gap between what the research protocol asks the agent to do ("propose ONE structural change: add a filter, change an exit, gate on a regime") and what it can write ("`feature('ema_7') > feature('ema_21')`") is the largest single capability ceiling on the platform, and it is not a harness problem.

Two smaller things also cost the model dearly: it learns what a rule does only by spending a counted trial on a backtest, and it authors in nested JSON, which a 7B mis-nests and a frontier model pads. The validator's errors are good (`path` + `message`, `strategy-validator/src/lib.rs:15`) but stop at "what is wrong".

## 1. Principles

- **P1 — Say it in the language a quant would.** The model writes strategies in SLv2 text (`entry long when crossed_above(ema(close,7), ema(close,21)) and rsi(close,14) < 60`), which compiles to the canonical JSON (ADR-0010: three front doors, one canonical form). JSON remains the storage and runtime form and stays writable for the MCP client; it stops being what a model is asked to produce.
- **P2 — One feature engine.** Backtest, live runtime, the `analysis` templates and the model-training frame all read features from DATA-006's engine (ADR-0029). A feature that exists anywhere exists everywhere, with the same PIT lag and warm-up.
- **P3 — Look before you spend.** Every rule can be previewed against stored bars with no trial: how often it fires, when, on what bars, and what the exits would have done. A trial is spent only on a strategy the model has already seen behave.
- **P4 — The validator teaches.** Every rejection carries `path`, `message`, and now `fix` (a concrete edit) and `see` (a guide anchor). Three rejections on the same path escalate to a worked example. Loosening is refused, not warned (the `rejects_loosening` test stands).
- **P5 — Change is a move.** Iteration happens through typed structural moves (`add_filter`, `change_exit_unit`, …) that produce a new version with a diff the model and the human can read in words. Free-form rewrites are allowed but recorded as `rewrite`, and the research protocol's "one change at a time" becomes checkable: a version with more than one move is flagged.
- **P6 — Discoverable, not memorised.** The model asks what features, families, exits and sizing rules exist and what they cost; nothing about the language lives only in the charter.
- **P7 — Same surface, every tier.** SLv2 is designed to be grammar-constrainable, so a `select_fill` tier writes it under a grammar and a frontier model writes it freely; both compile through one path.

## 2. The feature engine, phased by demand

[DO] Build DATA-006 §3–§7 as specified, with the catalogue landing in this order (each phase is a self-contained PR with the golden parity test DATA-006 §5 requires against the legacy `ema`/`rsi`):

| Phase | Families | Unlocks |
|---|---|---|
| F1 | arithmetic/math; `ret lag diff`; `rolling_*`, `ewma`, `zscore`, `pct_rank`; `sma ema rsi atr macd bb donchian keltner roc`; calendar | ~80% of retail-style rule ideas; ATR-unit exits; z-score mean reversion; session filters |
| F2 | range volatility (`parkinson … yang_zhang`), realised measures (`rv bv rq jump_share`), `hurst variance_ratio autocorr fracdiff`, `efficiency_ratio`, `lookback_ensemble` | regime gates, vol targeting, persistence filters, the `analysis` templates (AGENT-006 §5.2) reading the same code |
| F3 | liquidity/cost (`edge_spread roll_spread cs_spread amihud rel_volume`), order flow (`signed_volume trade_size_q ofi`) | cost-aware entries; `check_tradeable_frontier` and `cost_shock_curve` sharing the engine |
| F4 | `regime(model_ref)`, `prediction(series_ref)`, `text(series_ref)` | model-driven strategies through the MLOps registry, PIT-checked at the engine |

Each family is registered (DATA-006 §7) with: parameters and ranges, output dtype, warm-up bars, PIT lag, cost class (`cheap | rolling | model`), and a one-line "use when". That registry is what `list_features` (§5) serves.

## 3. SLv2 as the model-facing language (FEAT-004 §3, amended)

Keep the grammar in FEAT-004 §3.1 with these amendments, all in the direction of a small model writing it correctly:

- **Every clause on its own line, keywords first** (`entry`, `exit`, `size`, `risk`, `params`, `inputs`). A `select_fill` tier is given a JSON-Schema-shaped grammar over these clauses (one clause per fill), not the EBNF, and the compiler assembles the text. A frontier tier writes the text.
- **`inputs` are mandatory for anything used twice** and named; the compiler rejects an expression longer than 120 characters inline with `fix: "bind it as an input"`. This keeps rules legible to the human reviewer and to the model on re-read.
- **Units are explicit**: `exit stop 1.5 atr from entry`, `exit target 2 % from entry`, `exit time 48 bars`. A bare number in a distance is a compile error with the three legal units named.
- **`crossed_above/below`, `rising/falling(x, n)`, `between(x, a, b)`, `abs/min/max/clip`** are compiler-provided; the rising-edge semantics the v1 signal node had become explicit.
- **State predicates** (`flat`, `long`, `bars_in_trade`, `unrealized_return`, `last_exit_reason`) as specified; they are what make "re-enter only after a stop-out cooldown" expressible.
- **Typed `params` with ranges** as specified (they are what `run_sweep` sweeps; v1.2's `param('name')` remains an alias).
- **`order` clauses** as specified; default market.
- **Comments (`# …`) are part of the language and survive the round trip.** A strategy is
  read by a human reviewer, by the model on its next turn, and by whoever opens the dossier in
  six months, and none of them can recover *why* a threshold is 0.6 from the number alone. The
  parser attaches a comment to the clause that follows it, the compiler stores it on the
  canonical node, and the decompiler emits it back — so intent survives compile → JSON →
  decompile, which is exactly where it is lost today. `apply_move` carries the comment forward
  and appends the move's reason.
- **No user-defined functions, no loops, no multi-instrument** in v2.0 (OQ-040 stays open). Deliberately: everything above is grammar-constrainable, and anything Turing-ish is `run_python`.

[DO] `crates/strategy-lang`: parser (text → AST), compiler (AST → canonical `StrategyDefinition`, `crates/domain/src/strategy_def/mod.rs:64`), decompiler (JSON → canonical text, so every stored strategy has a readable form), and the clause-level JSON grammar for constrained tiers. The v1 expression grammar is kept as a subset the decompiler emits when nothing newer is used.

## 4. The authoring loop (the tools, AGENT-006 `strategy` namespace)

| Tool | Signature | What it gives the model |
|---|---|---|
| `list_features` | `(family?, cost_class?, query?) -> {features[{name, params, warmup, pit_lag, cost, use_when}], total}` | P6: what exists and what it is for |
| `explain_strategy` | `(strategy_id \| text) -> {prose, inputs[], entries[], exits[], size, risk, params[], warmup_bars, features_used[]}` | reads a strategy back in words; the decompiler plus a template |
| `compile_strategy` | `(text) -> {definition_json, warnings[], errors[{path, message, fix, see}]}` | P4; never persists |
| `validate_strategy` (exists, upgraded) | `(definition_json \| text) -> {ok, errors[{path, message, fix, see}], warnings[], features_used[], warmup_bars, estimated_cost_class}` | `fix` and `see` added; accepts text |
| `preview_signals` | `(text \| strategy_id, instrument, timeframe, start, end, max_events=50) -> {n_entries, n_exits, by_month[], sample_events[{ts, side, reason, bars_in_trade, exit_reason}], warmup_ok, never_fires_because?}` | P3: a dry run over stored bars through the feature engine and the exit policy, **no trial, no ledger row**; capped by `max_tool_result_bytes` with a handle |
| `diff_strategies` | `(a, b) -> {moves[{kind, path, before, after, words}], n_moves, is_single_change}` | P5: what changed, in words; AST diff classified into FEAT-004 §6 move kinds, `rewrite` when it cannot |
| `apply_move` | `(base_version, move: {kind, args}) -> {new_version, text, diff}` | the typed edit; one move, one version, validated |
| `create_strategy` (exists) | now accepts `text`; records `parent_version` and the move(s) that produced it | lineage for `family_test` and move memory |
| `suggest_moves` | `(strategy_id, diagnostics_ref) -> {moves[{kind, args, why, expected_effect}]}` | maps `get_diagnostics` findings ("loses in high-vol months") to candidate moves ("add_regime_gate on rv(20) pct_rank < 0.8"); deterministic rules first, model-ranked on capable tiers |
| `strategy_skeleton` | `(hypothesis_family: trend\|mean_reversion\|breakout\|carry\|vol_target\|model_signal) -> {text, params[], notes}` | a correct starting point per family, so the first version is never a syntax lesson |

**It says which clause blocked the entry.** `never_fires_because` at strategy level tells a
model that something is wrong; it does not tell it *what*, so the next move is a guess and the
guess costs a round trip. The preview evaluates each clause of each entry condition
independently over the window and returns a **blocking profile**: for every clause, the share
of bars on which it was true, and for the conjunction, the share on which it was the *last*
clause to fail. "`rsi(close,14) < 30` was true on 0.4% of bars and was the sole blocker on 91%
of the rest" is a finding a model can act on in one step, and it is the highest-value feedback
in the authoring loop — it turns "it doesn't fire" into "this threshold is the problem".

The same profile is what `suggest_moves` (§4) reads to motivate a `change_threshold` or
`remove_filter`, so a suggestion cites a measurement rather than a heuristic.

**`preview_signals` obeys the research cutoff and the sealed holdout.** This is the one
place where "no trial" could quietly become "no rules". A dry run that could read holdout
bars would let an agent tune against the holdout for free, one preview at a time, and
nothing in the gate funnel would ever see it — the cheapest possible route around the
platform's entire honesty stack, reached by the tool most designed to be called often.
So: `preview_signals` reads through the same point-in-time path and the same research
cutoff as a backtest (DATA-005); holdout partitions are outside its query scope exactly as
they are outside every tool's but `spend_holdout_look` (AGENT-006 §5.4); and a preview
window that overlaps the sealed holdout is **refused**, naming the partition, rather than
silently truncated to the allowed range — a silent truncation would make the preview
disagree with the later backtest and teach the model to distrust the tool.

The same applies to its sibling: `future_perturbation_test` is `preview_signals` run twice,
so it inherits the cutoff rather than restating it.

`preview_signals` is the tool that changes behaviour most: the research protocol's step 1 becomes "write → preview → adjust → create", and a strategy that never fires, fires every bar, or fires before its warm-up is caught before a trial. It composes with the leakage tools: `future_perturbation_test` (AGENT-006 §5.4) is `preview_signals` run twice with a shifted column and diffed.

## 5. The validator as teacher

[DO] `ValidationError` gains `fix: Option<String>` and `see: Option<String>` (a guide anchor like `guide#exits.units`). A table in `crates/strategy-validator/src/teach.rs` maps error codes to fixes, e.g.:

| Code | Message | Fix |
|---|---|---|
| `unknown_feature` | `rsi14` is not a feature | `did you mean rsi(close, 14)? list_features(query="rsi")` |
| `distance_without_unit` | `exit stop 1.5` has no unit | `write 1.5 atr / 1.5 % / 1.5 sigma from entry` |
| `when_names_expression` | `when` must name a node, not an expression | `bind the expression as an input, then reference it` |
| `loosening` | this change widens the risk envelope | refused; `tighten-only against the platform gate (guide#risk)` |
| `warmup_exceeds_window` | features need 1,000 warm-up bars; window has 600 | `start earlier or shorten the longest lookback (ema_200 → ema_100)` |
| `never_fires` (from preview) | no entry in 18 months | `loosen the threshold you set at …, or check the sign` |

Three rejections on the same `path` within a run return, additionally, a worked example from the guide for that clause. This is the cheapest "understanding" on a small tier: the model sees a correct instance, not a rule.

## 6. The guide

[DO] Rewrite `authoring_guide.md` sections "Strategy definition format", "Expression grammar" and "Features available" around SLv2: one page of grammar, one worked example per hypothesis family (the skeletons of §4), the units table, the moves table, and per-family feature notes generated from the registry. Served by `get_authoring_guide(section)` (AGENT-005 §3.3) so a 7B fetches one section at a time. The JSON form is documented in an appendix for the MCP client.

## 7. Build order

| Phase | Items | Why |
|---|---|---|
| 1 | Feature engine F1 + registry + `list_features`; `validate_strategy` gains `fix`/`see` | the model can express most ideas and is told how when it cannot |
| 2 | `preview_signals` over the v1 JSON form | trials stop being the first feedback |
| 3 | `crates/strategy-lang` parser/compiler/decompiler; `compile_strategy`, `explain_strategy`, `create_strategy(text)`; guide rewrite | the language |
| 4 | `diff_strategies`, `apply_move`, lineage on `create_strategy`; `suggest_moves` (rules) | one change at a time becomes mechanical |
| 5 | F2 families; clause-level grammar for `select_fill` tiers; `strategy_skeleton` | the 7B writes strategies under a grammar |
| 6 | F3, F4 families; `suggest_moves` model-ranked on capable tiers | cost-aware and model-driven strategies |

## 8. Acceptance

1. Every feature the backtest accepts is served by `list_features`, and every feature in `list_features` is accepted by the backtest, the live runtime and the `analysis` templates (one engine).
2. The trend, mean-reversion and vol-target skeletons compile, validate and preview on BTC-USD 1h with non-zero entries and no warm-up violation.
3. A strategy written in SLv2, compiled to JSON, decompiled to text and recompiled yields an identical definition (round-trip test).
4. `validate_strategy` on the six error cases in §5 returns the listed `fix`; a third identical rejection returns a worked example.
5. `preview_signals` writes no trial row, and its `sample_events` match the fills of a subsequent backtest on the same window for the first ten events.
5a. `preview_signals` over a window overlapping the sealed holdout is refused and names the partition; it cannot read a bar a backtest could not read. Adversarial: the refusal is asserted, not just the happy path.
6. `diff_strategies` classifies every FEAT-004 §6 move kind from its own `apply_move` output, and `is_single_change` is false for a two-move version.
7. On `qwen2.5-7b`, a research goal produces a compiled SLv2 strategy through the clause grammar in ≤ 6 fills with no `unknown_feature` error, on the eval dataset (AGENT-009 `research` suite).
8. The authoring guide's SLv2 page is ≤ 4 KB per section and every `see` anchor resolves.

## 9. Non-goals

- Multi-instrument / cross-sectional strategies (OQ-040) — after v2.0.
- User-defined functions or loops in SLv2 — that is `run_python`.
- Changing the canonical JSON storage form — ADR-0010 stands.
- A visual builder; the MCP draft builder remains for that client.

## 10. Open questions

- Whether `suggest_moves` should ever propose a move the diagnostics did not motivate (exploration) or only motivated ones; ship motivated-only, let the research protocol's Experiment step be where exploration happens.
- The cost class of realised measures (`rv` on 1m sub-bars for 1h strategies) at preview time; may need caching per instrument/window.
