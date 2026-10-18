# Phase 5 — The language

**Completion: 80% (12 / 15 tasks), plus 3 `PARTIAL` — re-counted 2026-10-04 (track VERIFY) from the per-task markers; it read 5 / 15 on 2026-09-27:  the compiler and the round trip close on their literal acceptance, and the validator pass that refused every sweepable strategy is fixed. The feature engine is built; the language is authorable but not yet previewable.**

> **This file was wrong about the engine and right about the language.** Until 2026-09-16 it
> recorded all fifteen tasks as `NOT STARTED`. `crates/features` is in fact about 5,100 lines
> over 101 passing tests, and `crates/backtest/src/sim.rs` resolves strategy features through
> it — so the backtest has supported around fifty families for some time.
>
> **The agent could not see any of them.** The MCP server's `initialize` instructions told
> every model, every session, that "backtests support only ema_N and rsi_N features". That
> sentence was false and it was the phase's real bottleneck: a model told it has two
> indicators uses two indicators. `list_features` now serves the registry the simulator reads,
> and the instructions point at it.
>
> **This line was false.** `crates/strategy-lang` exists and always did; what
> was true is that nothing depended on it. The original read: ~~`crates/strategy-lang`
> does not exist, so S-5.5 … S-5.9, S-5.12, S-5.13 and S-5.15 are~~
> genuinely not started. The highest-value task that needed no language was **S-5.4,
> `preview_signals`**, and it shipped 2026-09-16: a model can now look at what a draft does on
> a window without buying a backtest, and a window reaching into the sealed holdout is refused
> by name rather than quietly shortened.
>
> One finding worth acting on: phase 3 implemented Parkinson, Garman-Klass and Rogers-Satchell
> volatility in Python templates while `crates/features` computes the same families in Rust.
> **Two implementations, nothing comparing them.** S-5.14's acceptance already asks for that
> comparison; it is now a reconciliation test this repository needs rather than a future
> nicety.

> **Two things found on 2026-09-21 that are worth reading before touching this
> crate.**
>
> **`FormStrategy` is now the only writer of SLv2 text that no human reads before
> it compiles**, so every document-level rule in `parse::lint` needs a matching
> pass in `to_text`. `used_twice` was one, and it cost a model two fills to
> produce a refusal it could not have avoided from inside either fill. Nothing
> proves the form can only produce documents the language accepts — that is the
> gap to watch.
>
> **The constraint clause was a free-form string.** `compile` stored a
> `constraint` clause's text verbatim and never read it, so `constraint blah`
> compiled into a stored definition and failed much later inside
> `domain::strategy_def::params::parse_constraint` — at sweep time, on a strategy
> somebody had already committed to, with the error naming a place the author was
> no longer looking. `domain` now exports `check_constraint` (the answer, not the
> parse tree — `Operand` stays internal) and the compiler refuses it where it is
> written, against the same reader the sweep uses.

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

### The crate existed all along, and nothing imported it, 2026-09-21

**`strategy-lang` 129 → 141 tests; `mcp-server-lib` 197 → 209.**

Seven task entries in this file said *"`crates/strategy-lang` does not exist"*.
It exists, at **13,877 lines over 129 passing tests**, with `diff`, `lineage`,
`suggest_moves` and `blocking_profile` already built. **And `grep strategy-lang
crates/*/Cargo.toml` matched only its own manifest** — nothing in the workspace
depended on it. Thirteen thousand lines of built-but-unreached, described in the
plan as absent. Four tools now reach it: `compile_strategy`, `explain_strategy`,
`diff_strategies`, `apply_move`.

**`apply_move` genuinely did not exist** and is new (~900 lines, 12 tests). Two
decisions are worth keeping:

- **`Edit` does not name a `MoveKind`.** The differ is the arbiter of what a
  change *is*; an applier that accepted a kind could disagree with it, and a
  caller asking for `swap_signal` while supplying a threshold tweak would get a
  lineage saying something its author does not believe.
- **The single-move guarantee is the type.** `Applied` has private fields and no
  public constructor; `apply_move` runs the differ on its own output and refuses
  `0` changes or `n > 1`. `Applied::kind()` is a **measurement**, never an
  unchecked claim. Addresses are newtypes, so `entry.long.0` is an error rather
  than a silent clause zero.

### The one that stops the whole authoring path

**The validator refuses every document the language can compile, and the fix
text attached to the refusal names the version it refuses.**

`strategy-validator/src/schema.rs:22` accepts only `"1.0" | "1.2"`. The fix in
`teach.rs:94` for that same code reads: *"1.0 is the base, 1.2 adds typed
parameters, **1.3 adds the exits block**."* And the compiler targets **v1.3** —
it must, because `exits` is the only schema that can hold
`exit stop 2 % from entry`, which `domain` documents REQUIRED at 1.3.
`routes/strategies.rs` runs `validate()` before it persists. **So the SLv2
authoring path is dead at its last step**, and `create_strategy(text)` cannot be
built until it is decided. Pinned by a test asserting the version is the *only*
complaint — which will fail the day it is fixed, and the repair then is to
delete the test, not widen it. **Neither reading implemented.**

### What the live registry says, and one correction to the track's own figure

The decompiler was run over **all 32 stored definitions**, which S-5.7's note
said nobody had ever done:

```
TOTAL 32 | READABLE 1 | UNREADABLE 13 | NOT A StrategyDefinition 18
```

**One stored strategy in the entire database has a readable SLv2 form.** The 13
refusals are honest and named; the dominant one — *"an action fires on the
signal …"*, 7 of 13 — is a single shape SLv2 cannot express, **and it is what
this platform's existing strategies actually look like**.

**The track reported "all 32 are `definition_version = 1.0`". That is wrong, and
the real distribution is more interesting:** 18 have no version key at all (the
empty rows), **9 are 1.0, 4 are 1.2, and 1 is `1.1`**. Zero are 1.3 and zero
carry an `exits` block, so the substantive point stands — nothing the language
can produce has ever reached the store — but there is **a live row at `1.1` that
the validator's own `matches!("1.0" | "1.2")` refuses**, which no one has
noticed.

**18 of the 32 rows are test garbage — 56% of the live strategy registry.**
`definition_json = '{}'`, `head_revision IS NULL`, slugs shaped `s_<32 hex>`:
residue from `crates/ledger/tests/pg_strategy_revision.rs`, which connects with
`DATABASE_URL` from `.env` — **the platform's own database** — and never tears
down. The tests are using the `head_revision IS NULL` escape hatch that
migration `0058` left open for its own backfill **as a normal write path**, and
all of it is visible to `list_strategies`.

### Two more open questions, neither implemented

**Lineage.** `parent_version` does not exist anywhere in the repo. Lineage is
`strategy_revision.parent_hash`, derived server-side from the `head_revision` of
**the same `strategy_id`** — while `create_strategy`'s own description advises
*"mint versioned slugs (`my_strat_v2`) when iterating"*. A new slug has no head,
so `parent_hash` is NULL; the only way to get a parent is to reuse the slug,
which overwrites the head the advice warns about. **Measured: 40 agent-authored
revisions across 34 slugs, 6 with a parent — 85% of the agent's authoring
history has no lineage at all.**

**Authorship.** `routes/strategies.rs` binds `actor_kind = "human"`
unconditionally, while migration `0058` says that column *"distinguishes a human
author from the research agent, which is the difference between a hypothesis and
a sample"*. The live table holds 40 `'agent'` rows, so a second writer exists
and the two paths disagree about who wrote a revision.

### Blocked, with the exact missing symbol

**`suggest_moves` (S-5.11) is built and tested, and all three of its inputs have
no producer.** `blocking` needs `ClauseCounts { bars_true, bars_sole_blocker }`
per clause — `SignalPreview` emits none. `regimes` needs `by_regime`, documented
*"Empty until a regime engine supplies labelled windows"*, and the nearest
alternative carries a free-form `detail: String`. `exit_reasons` needs a symbol
that **does not appear anywhere in `crates/backtest`**. A `suggest_moves` tool
would return zero suggestions on every real run, for ever. Not shipped.

**S-5.15 is blocked, and this file's reason was the wrong one.** It says intent
surviving the round trip "needs the round trip". **The round trip works** — six
green tests including a corpus-wide pass and a text fixpoint. The real blocker:
`domain::strategy_def` has **no comment field on anything**. The parser already
attaches comments and the compiler drops them because there is nowhere to put
them.


## The feature engine

### S-5.1 F1 families, inside the existing engine — L
`DONE` 2026-09-16 — the F1 families are in the registry and resolve, and **the golden parity test exists and passes**.

This entry was wrong in the same direction as the rest of the file. `crates/features/tests/golden_legacy_parity.rs` is 339 lines: three instruments (trending, mean-reverting, volatility-clustered) against three timeframes (1m, 5m, 1h), comparing `to_bits` against transcriptions of the pre-change source rather than re-derivations of the formulas — tolerance zero, because `consistency::diff_serve` compares stored values bit for bit and any drift would surface as `Diagnosis::CodeChanged` on every historical serve.

What was genuinely missing was narrower and hidden by the test's shape: it names legacy features one at a time, so it covered `momentum_n` and missed `ret_n`, `logret_n` and `sma_n` — a hand-written list covers what its author remembered. `every_alias_is_bit_identical_to_the_family_it_aliases` now reads the aliases out of the registry, so a family that gains one is covered the day it does, and asserts the pair declare the same warm-up as well as the same values.

Audited 2026-09-16 against the source. `crates/features` is a real engine — about 5,100 lines over 101 passing tests, with `registry.rs` (583), `runtime.rs` (1,068) and `leakage.rs` (1,301) — and `crates/backtest/src/sim.rs:987` resolves every strategy feature through `features::runtime::feature`. Proven by construction rather than by reading: `atr_14`, `parkinson_vol_20`, `rolling_std_30`, `donchian_upper_20`, `obv` and `hour_sin` all resolve and run.

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
`DONE` 2026-10-03 (track LANG, wave 2) — the requirements gate now resolves through the registry: `backtest::requirements::parse_feature` calls `features::registry::resolve` and takes the warm-up from `features::runtime::lookback_bars` (building on the working-tree sma/atr edit; `every_family_in_the_registry_is_derivable_with_the_runtimes_warmup`, `backtest --lib` 451 pass). `list_features` pages with `offset`/`next_offset` (`paging_reaches_every_family_exactly_once`). With `features_registry_parity.rs` (5 tests) the engine, the registry, the backtest gate and `list_features` accept the same names; the live runtime reads the same `features::runtime`. The analysis-template clause is met for the families the templates share with the engine (reconciliation tests): the templates are numpy and read no registry. No migration (ADR-S5-10). The earlier finding, kept for the record: the gate used to accept only ema/rsi.

This was the phase's real bottleneck and it was not a missing engine. The server's `initialize` instructions told every model at the start of every session that "backtests support only ema_N and rsi_N features" while the engine resolved around fifty families. A capability ceiling written in prose is still a ceiling. That sentence now points at `list_features` instead.

Still missing: `migrations/0082_feature_registry.sql`, so nothing persists a registry version, and the second half of the acceptance — every feature the backtest accepts is served, **and** every feature listed is accepted — is proven in one direction only.

Audited 2026-09-16 against the source. `crates/features` is a real engine — about 5,100 lines over 101 passing tests, with `registry.rs` (583), `runtime.rs` (1,068) and `leakage.rs` (1,301) — and `crates/backtest/src/sim.rs:987` resolves every strategy feature through `features::runtime::feature`. Proven by construction rather than by reading: `atr_14`, `parkinson_vol_20`, `rolling_std_30`, `donchian_upper_20`, `obv` and `hour_sin` all resolve and run.

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
`DONE` 2026-10-03 (track LANG) — all six §5 codes now have a real producer and carry the teaching table's fix: `unknown_feature`, `when_names_expression`, `loosening` (validator), `distance_without_unit` (the SLv2 parser; its fix and the table's now teach the same `%` / `atr(N)` / `sigma(N)` syntax — the table still said bare `atr`), `warmup_exceeds_window`, `never_fires` (preview). `the_six_teachings_have_producers.rs` 7/7; mutation: deleting `atr(` from the table fix fails it. The "third rejection returns an example" clause is met by attaching the example to every rejection (ADR-S5-05).

`ValidationError` gains `code`, and `fix`/`see` are **methods over it** rather than fields — a stored fix is a second copy of the table a caller can forget to fill in, and the forgetting is silent because the error still renders. `teach.rs` carries §5's six plus the twelve this crate constructs, and `every_code_the_validator_constructs_can_teach` scans `src/*.rs` for `code: "…"` rather than walking a registered list, skipping `teach.rs` itself (the self-referencing scan trap, hit again).

**The half that was actually broken was downstream.** `ValidationErrorItem` — what the MCP tool returns — mapped `{path, message}` and dropped the rest, in two identical copies (`authoring.rs` and `builder.rs`). The validator would have carried a fix for every code and no caller would ever have seen one. Now one `From` impl.

**And the pass it added carried a defect of its own for eleven days, found and fixed 2026-09-27.**
`feature_names::check` ran on the **raw** definition while every other pass read the copy
materialized with parameter defaults, and `features::resolve_feature("ema_{{fast}}")` is `None` — so
**every definition with a swept indicator period was refused `unknown_feature`**. `{{param}}`
interpolation is the whole of v1.2, `compile.rs` emits it for any `input x = ema(close, fast)`, three
of the four worked skeletons use it, and the persist route runs `validate()` first. A hard-coded-period
strategy validated and the same strategy made sweepable did not. The old refusal also offered no
repair: `ema_{{fast}}` is twelve characters against `ema_20`'s six and the edit-distance cap is three,
so it fell through to "Call `list_features`". The pass now reads the substituted copy and is skipped
when substitution fails, so `ema_{{nope}}` is reported as a *parameter* error rather than pointing at
the wrong clause. `tests/a_sweepable_strategy_is_not_an_unknown_feature.rs`: 3 of its 5 cases fail
before the change, 0 after.

**And it found a real defect.** The validator never checked that declared feature names resolve: a definition with `rsi14` returned `Ok` and failed inside `features::runtime::feature` during the run — for an agent, after a trial was spent. `feature_names::check` resolves every name through the registry and suggests the repair, tried against `resolve_feature` rather than against the advertised examples (the example for `rsi` is `rsi_20`, so an edit-distance search offered nothing for `rsi14`, while `rsi_14` resolves).

**What keeps it `PARTIAL`: one of §5's six codes has no producer.** Five do — `unknown_feature`, `when_names_expression` and `loosening` from validator passes, and `warmup_exceeds_window` and `never_fires` from `preview_signals`, whose `diagnose` now returns `(code, prose)` and whose `SignalPreview` carries `never_fires_code`. The sixth, `distance_without_unit`, belongs to SLv2's grammar, which does not exist; it stays in the table with nothing emitting it, honestly.

One deviation, deliberate: the worked example is attached on **every** rejection rather than the third. §5 counts rejections within a run; this crate is a pure function of one document and has no run, and withholding the example until a counter it cannot keep would mean never attaching it. `rejects_loosening` is untouched and green.

Audited 2026-09-16 against the source. `crates/features` is a real engine — about 5,100 lines over 101 passing tests, with `registry.rs` (583), `runtime.rs` (1,068) and `leakage.rs` (1,301) — and `crates/backtest/src/sim.rs:987` resolves every strategy feature through `features::runtime::feature`. Proven by construction rather than by reading: `atr_14`, `parkinson_vol_20`, `rolling_std_30`, `donchian_upper_20`, `obv` and `hour_sin` all resolve and run.

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
`DONE` 2026-09-16 — `crates/backtest/src/preview.rs` walks the window; `crates/api/src/routes/agent_preview.rs` serves it at `POST /api/agent/strategy/preview`; `tools::strategy::preview_signals` declares it with `spends_trial: None`.

All three acceptance criteria are met and tested. **No trial row**: the route touches no ledger, and the spec says so where a reader will see it. **`sample_events` match a subsequent backtest's fills**: `crates/backtest/tests/preview_matches_simulator.rs` runs the same definition through `preview_signals` and `run_simulation_detailed` on one fixture and asserts every position the simulator opened was opened on a bar the preview called an entry — five for five, well inside the "first ten events" the acceptance asks for. **The holdout refusal is asserted**: five tests over `judge_window`, including the boundary at the cutoff second.

Three things the acceptance test had to learn about the simulator, each of which had made an earlier version of it wrong:

- `DetailedOutcome::trades` is a list of **closed round trips**, not entries — `Trade` carries an `exit_time`. A definition with no exit policy opens a position that never closes and reports zero trades however often it fired, so the first version of this test compared 1 against 0 and blamed the fixture. The fixture now declares `MaxHoldingBars`.
- **`ema_N` warms up over `5N` bars**, not `N`. The original 86-bar fixture put every crossing inside `ema_10`'s 50-bar warm-up.
- The simulator places an order only where an action's `on_signal` matches. The preview counted every rising edge, which would have promised entries to a definition carrying a signal it does not trade. Fixed — see ADR-S5-02.

**One deviation from the body text above, recorded as ADR-S5-01**: sample events carry the feature values that caused the firing, not an exit reason. An exit reason needs position state, which needs the exit tracker, which is the simulator; a preview that ran it would be a second simulator with its own answers about fills and holding periods. The declared exit policy is reported instead, which answers the question a preview can answer honestly — did you remember to write exits.

The holdout rule is ADR-S5-03, and it deliberately contradicts `Project::clip`'s own documentation: the cutoff clips, the holdout refuses.

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
`DONE`

~~`crates/strategy-lang` **does not exist**.~~ **False as of 2026-09-21, and it was false when it was written under seven separate tasks.** The crate is **13,877 lines over 129 passing tests** — `lex, ast, parse, compile, decompile, diff, suggest, block, form, skeleton, catalogue, diagnostic`. What was true is worse and more specific: **nothing in the workspace depended on it.** `grep strategy-lang crates/*/Cargo.toml` matched only its own manifest. Thirteen thousand lines of the built-but-unreached shape. See the note at the head of this file.

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

**Built 2026-09-21.** `crates/strategy-lang/src/{lex,ast,parse}.rs`. Clause per
line, comments attached to the clause below them, and **line-level recovery**, so
three bad clauses come back as three diagnostics rather than as the first one.

Both acceptance criteria are tested rather than described: a bare number in a
distance returns `distance_without_unit` whose `expected` list is exactly
``["`%`", "`atr`", "`sigma`"]``, and an over-long inline expression returns
`expression_too_long` with `fix: "bind it as an input, then reference it by
name"`. Every failure names its span and what was expected — a parser whose
errors do not teach is a parser a model cannot iterate against, which is the
whole argument of AGENT-011 §3.

### S-5.6 The compiler — M
`DONE` 2026-09-27 — the acceptance is now pinned at the byte level for all three skeletons, and
the one clause that was genuinely missing is closed. `tests/the_json_a_human_would_have_written.rs`
runs each skeleton through `domain::strategy_def::revision::canonical_json` — the same function the
revision hash is taken over — against hand-written JSON, so a `Decimal` that starts serializing
`0.0150` instead of `0.015`, or a `DistanceUnit` that stops being skipped at its default, fails.
A fourth test asserts the three pinned are the three the acceptance names. What it does not catch
is stated in the file: the expected side is deserialized before canonicalising, so anything the
typed deserializer normalises is normalised on both sides. FEAT-004 §3.1's `order` clauses now
parse; `order market` compiles to the same definition as its absence, because FEAT-004 names market
as the default and `OrderSpec` has no order-type field — said out loud rather than silently accepted,
with `limit`/`stop`/`stop_limit` and a stray `offset` or `tif` refused by name.

~~`crates/strategy-lang` **does not exist**.~~ **False as of 2026-09-21, and it was false when it was written under seven separate tasks.** The crate is **13,877 lines over 129 passing tests** — `lex, ast, parse, compile, decompile, diff, suggest, block, form, skeleton, catalogue, diagnostic`. What was true is worse and more specific: **nothing in the workspace depended on it.** `grep strategy-lang crates/*/Cargo.toml` matched only its own manifest. Thirteen thousand lines of the built-but-unreached shape. See the note at the head of this file.

AST → the canonical `StrategyDefinition` (`crates/domain/src/strategy_def/mod.rs:64`). JSON
remains the storage and runtime form (ADR-0010); it stops being what a model is asked to
produce.

**Acceptance:** a compiled definition is byte-identical to the equivalent hand-written JSON
for the three skeletons.
**Traces:** AGENT-011 §3, P1.

**Built 2026-09-21, and the gaps are named rather than approximated.**
`crates/strategy-lang/src/compile.rs` targets
`domain::strategy_def::StrategyDefinition` at v1.3; the trend skeleton compiles to
a definition structurally equal to the hand-written JSON
(`the_trend_skeleton_compiles_to_the_definition_a_human_would_have_written`).

**The load-bearing discovery: booleans are already numbers in this platform**, and
it is the only reason `and`/`or`/`not` compile without touching `domain`. The
frozen v1 grammar is `expr = comparison` with no combinators, but
`strategy-validator::expressions::check_primary` accepts a parenthesised
comparison and both the bytecode and the interpreter push exactly `1.0`/`0.0` for
every comparison operator. So `and → (A * B)`, `or → ((A + B) > 0)`,
`not → (1 - A)` — **no new bytecode op and no schema change**.

The normalisation is what makes it correct: every compiled condition must
evaluate to exactly `0.0` or `1.0`. `or` is `((A + B) > 0)` rather than the
shorter `(A + B)` because the short form is truthy at the top level and wrong one
level in — `not (a or b)` with both true becomes `1 - 2 = -1`, which the runtime
reads as **true**. `normalises_or_so_that_not_of_it_is_correct` pins it.

**What is refused by name rather than guessed at**, each naming the missing
mechanism: `atr`/`sigma` exit distances (Q-S5-01 — the schema gained the field
2026-09-22, so the refusal is narrower now: the grammar parses `atr`/`sigma` as
a bare token with no period, and `DistanceUnit::Atr { period }` needs one from
somewhere the parser does not have; the fix text says so rather than the
now-false "the schema does not have this yet"), `crossed_above`/`below` and
`rising`/`falling` (Q-S5-02 — the stated reason is **false as of 2026-09-27**:
`crates/strategy-runtime` has `Op::{CrossedAbove, CrossedBelow, Rising, Falling}`,
`HistoryRegisters` and `SignalEvaluator`. The refusal stands on what is still true —
`strategy-validator::check_call` accepts only `feature` and `bar`, and `backtest/src/sim.rs:1218`
still evaluates conditions with the *stateless* `evaluate_signals`, which answers `NeedsHistory`
rather than an answer — so a crossing strategy would validate nowhere and backtest nowhere, and
widening the grammar would be a broken promise. The message was corrected in place and is pinned
by `crossed_above_is_refused_for_the_two_reasons_that_are_still_true`), state predicates, several maths builtins,
`cooldown`, `vol_target`/`kelly` sizing and `max_daily_loss`.

~~**What is genuinely missing:** FEAT-004 §3.1's `order` clauses hit `unknown_clause` rather than a named refusal, and the acceptance asks for three skeletons where only AGENT-011 §3's trend skeleton exists.~~ **Both closed 2026-09-27** — see the completion note at the head of this task. Recorded while closing it: `domain::strategy_def::nodes`' frozen-grammar EBNF, the validator, and the runtime's `bytecode::compile` are now **three disagreeing implementations** of one grammar with nothing comparing them.

**One defect found and removed at integration.** `catalogue.rs` shipped from the
track carrying its own 61-family transcription of `features::registry` — the
two-implementations shape this repository keeps finding, and the track flagged it
itself. It now reads `features::registry::families()` directly, aliases dropped
(`resolve` rewrites them before construction, so admitting both spellings would
make the decompiler guess which the author wrote), with
`the_catalogue_is_the_engine_s_registry_and_holds_nothing_else` comparing the two
sets name by name and parameter by parameter.

### S-5.7 The decompiler and the round trip — M
`DONE` 2026-09-27 — re-read against its literal acceptance, which is "SLv2 → JSON → text → JSON
yields an identical definition (round-trip test)", and `tests/round_trip.rs` is exactly that over 13
definitions plus a text fixpoint. `REQUIRED_SHAPES` (30 entries) fails the build if the corpus stops
exercising any exit-rule kind, size mode, side, parameter type, boolean operator, or non-default
version/asset-class/trust path, so the suite cannot pass by covering less. The gap recorded below —
nothing has run the decompiler over the *stored* set — is the body's "every stored strategy has a
readable form", not the Acceptance line, and it stays recorded as an open test gap.

~~`crates/strategy-lang` **does not exist**.~~ **False as of 2026-09-21, and it was false when it was written under seven separate tasks.** The crate is **13,877 lines over 129 passing tests** — `lex, ast, parse, compile, decompile, diff, suggest, block, form, skeleton, catalogue, diagnostic`. What was true is worse and more specific: **nothing in the workspace depended on it.** `grep strategy-lang crates/*/Cargo.toml` matched only its own manifest. Thirteen thousand lines of the built-but-unreached shape. See the note at the head of this file.

JSON → canonical text, so every stored strategy has a readable form and the v1 expression
grammar remains a subset the decompiler emits when nothing newer is used.

**Acceptance:** SLv2 → JSON → text → JSON yields an identical definition (round-trip test).
**Traces:** AGENT-011 §3; §8.3.

**Built 2026-09-21.** `crates/strategy-lang/src/decompile.rs` plus
`tests/round_trip.rs`. `compile(parse(decompile(d))) == d` over 9 text-derived and
4 hand-built definitions, **and a text fixpoint**: `decompile` is stable on the
second pass, so a stored strategy's readable form does not churn between reads.

`REQUIRED_SHAPES` (30 entries) fails the build if the corpus stops exercising any
exit-rule kind, size mode, side, parameter type, boolean operator, or the
non-default version/asset-class/trust paths — the guard against a round-trip
suite that passes because it stopped covering anything.

Decompilation **refuses by name and never silently lies** for AI/universe/LLM
node kinds, `SizeMode::Model`, per-signal sizing, a declared-versus-used feature
mismatch, unknown lanes and dead condition nodes. `deencode` reads the three
boolean shapes back exactly and refuses to rewrite `(A > 1) * 2`, because a
definition that really multiplies a flag by two means that.

**What is not covered:** "every stored strategy has a readable form" is true of
every shape the corpus holds, not of every definition in the registry — nothing
has yet run the decompiler over the stored set.

### S-5.8 The authoring tools — M
`DONE` 2026-10-03 (track LANG) — `validate_strategy` and `create_strategy` take SLv2 `text` (and a `form`) as well as a definition and end at the same validator and save route; `validate_strategy` returns `features_used`, `warmup_bars` and `estimated_cost_class`; a v3 header is refused by name (`this_is_slv3`). `explain_strategy` names every input, entry clause, exit, the size rule and the warm-up, including for swept strategies (it returned null for every skeleton before), and shows comments. Lineage: saving under the header's name chains onto the previous head — tested at the tool against a stub platform (`saving_the_same_name_twice_…`); the derivation itself is the ledger's (ADR-D-21). `tools/strategy_lang.rs` tests all pass. Not covered: recording the producing move beside the revision (no field on the route).

~~`crates/strategy-lang` **does not exist**.~~ **False as of 2026-09-21, and it was false when it was written under seven separate tasks.** The crate is **13,877 lines over 129 passing tests** — `lex, ast, parse, compile, decompile, diff, suggest, block, form, skeleton, catalogue, diagnostic`. What was true is worse and more specific: **nothing in the workspace depended on it.** `grep strategy-lang crates/*/Cargo.toml` matched only its own manifest. Thirteen thousand lines of the built-but-unreached shape. See the note at the head of this file.

`compile_strategy` (never persists), `explain_strategy` (reads a strategy back in words),
`create_strategy(text)` recording `parent_version`, and the upgraded `validate_strategy`
accepting text.

**Acceptance:** `explain_strategy` names every input, entry, exit, size rule and the warm-up
bars; `create_strategy` records lineage for `family_test`.
**Traces:** AGENT-011 §4.

### S-5.9 A grammar for the small tiers — M
`PARTIAL` 2026-10-04 (S-CATALOGUE track) - Re-checked, no status change. The unmet half is harness-side (handing a `select_fill` tier the form grammar as its decoding schema in `crates/harness/src/drive.rs` / `local_driver.rs`, both owned by the runtime track) and the AGENT-009 research-suite run needs a live model.
`PARTIAL` 2026-10-03 (track LANG) — measured on a real model for the first time: qwen2.5:7b-instruct through local Ollama, 8 hypothesis goals, one constrained fill per clause (header, entry, size, two exits). **Fills: 5 per goal, never over 6; `unknown_feature`: 0 of 8** (both acceptance clauses met). Compiled: 3 of 8 with the compact schema (every failure `wrong_arity`, which it gives up by design), 5 of 8 with the exact per-family schema (two failures were the sampler failing on its ~175 KB grammar). A first run compiled 0 of 8 because the grammar let a filter compare two numbers and a stop be negative; both are now excluded. The form is reachable: a `form` door on `validate_strategy`/`create_strategy`, and the grammar via `get_slv2_guide(section="form.schema")` (the 1,200-byte descriptor budget forbids it as a tool's own schema, and the per-family schema is ~270 KB; ADR-S5-11). **Still unmet:** not run on AGENT-009's research suite, and nothing hands a `select_fill` tier the grammar as its decoding schema (harness side).

~~`crates/strategy-lang` **does not exist**.~~ **False as of 2026-09-21, and it was false when it was written under seven separate tasks.** The crate is **13,877 lines over 129 passing tests** — `lex, ast, parse, compile, decompile, diff, suggest, block, form, skeleton, catalogue, diagnostic`. What was true is worse and more specific: **nothing in the workspace depended on it.** `grep strategy-lang crates/*/Cargo.toml` matched only its own manifest. Thirteen thousand lines of the built-but-unreached shape. See the note at the head of this file.

A clause-level JSON-Schema grammar (one clause per fill) rather than the EBNF, so a
`select_fill` tier writes SLv2 under constrained decoding and the compiler assembles the
text.

**Acceptance:** on `qwen2.5-7b`, a research goal produces a compiled SLv2 strategy in ≤ 6
fills with no `unknown_feature` error, on the eval dataset.
**Traces:** AGENT-011 §3, P7; §8.7.

**Built 2026-09-21.** `crates/strategy-lang/src/form.rs`: typed clause fills with
serde, a JSON Schema for a constrained decoder, `to_text()`, and
`FormStrategy::from_strategy` — the projection of free-form SLv2 back into the
form.

**One compiler, asserted.** `FormStrategy::compile()` is `to_text()` +
`text_to_definition`, and
`the_form_compiles_through_the_language_and_not_beside_it` holds the two paths
equal. Two compilers would mean a strategy that validates in the form and not in
the language.

**The schema emits one branch per family, read from `Catalogue::engine()`**, each
carrying its own arity and each parameter's range — so `unknown_feature` and
`wrong_arity` are *unreachable* for a constrained decoder rather than
discouraged. `every_family_in_the_schema_compiles_when_the_form_uses_it` walks all
57 branches.

**13 narrowings are named** with a typed `Limitation`, each carrying an SLv2
document and a `Reach` saying what the language does with it — `Compiles` (the
form is strictly narrower) or `RefusedByTheCompiler(code)` (writing it as text
will not help). `every_limitation_is_real_and_reachable` checks **both halves
against the live compiler**, so an entry that stopped being true fails the build.

**One spec conflict resolved a third way, flagged so it can be overruled.** §3
gives the "anything used twice must be an input" rule to the compiler *and* gives
a `select_fill` tier one clause per fill — and a fill has no view of the other
fills, so two independently correct fills can each name `ema(close, 7)` and the
assembled text is refused **after both fills are spent**, on the tier least able
to afford it. The assembler **binds** rather than failing: a call matching a
declared input is written as that input's name, and a call written twice is
hoisted into a generated one. It is the substitution `decompile` already performs
in the other direction, so it changes nothing about what the strategy means.

**What keeps it `PARTIAL`:** the acceptance is "on `qwen2.5-7b`, ≤ 6 fills, no
`unknown_feature`, on the eval dataset". The mechanical half is proved — a whole
strategy is 4 fills and the schema cannot name a feature the engine does not
serve — but nothing here ran a model.

### S-5.10 Moves, diffs, lineage — M
`DONE` 2026-10-03 (track LANG) — `apply_move` and `diff_strategies` were already built and served; what was missing was the acceptance at the surface a model reaches. `tools/moves_roundtrip.rs` (4 tests): each of eight edit kinds goes through the `apply_move` tool and its output through the `diff_strategies` tool, which returns the same kind and `is_single_change` true; a two-move version is `false` with `n_moves` 2; an edit that changes nothing and a malformed address are refused.

`diff_strategies` classifies an AST diff into the FEAT-004 §6 move kinds and says what
changed **in words**, with `is_single_change`; `apply_move` produces one new validated
version per move. This makes the research protocol's "change one thing at a time"
checkable rather than exhorted.

**Acceptance:** every move kind round-trips from `apply_move` through `diff_strategies`;
`is_single_change` is false for a two-move version.
**Traces:** AGENT-011 §4, P5; §8.6.

### S-5.11 `suggest_moves` — S
`PARTIAL` 2026-10-04 (S-CATALOGUE track) - `exit_reasons` now has a producer: the simulator's stored result carries `exit_reasons` (`[{reason, trades, share}]`, shares sum to one, trades no declared exit closed under `signal_exit`; absent when no exits were declared) via `DetailedOutcome::result_document` (`sim::tests::the_stored_document_says_which_exit_rule_closed_how_many_trades`), and `suggest_moves` takes `run_id` and reads it from `GET /api/backtests/{id}` (`exit_reasons_from` says what was read, or why nothing was). Still unmet: `regimes` has no producer (needs a regime label to gate-predicate mapping per strategy); the live read of `/api/backtests/{id}` is not driven end to end.
`PARTIAL` 2026-10-03 (track LANG) — `suggest_moves` is now a tool (namespace `analysis`), measuring its own blocking profile over a window (ADR-S5-08) and returning every suggestion with a typed `why` carrying the numbers it was read from (tested: each has `diagnostic`, `measured`, `says`). `exit_reasons` and `regimes` are accepted from the caller (ADR-S5-13) because nothing produces them typed; a regime gate is checked against the registry. **Unmet:** those two inputs have no producer in the platform, so the rules that read them fire only on caller-supplied findings.

Maps `get_diagnostics` findings to candidate moves with a reason ("loses in high-vol months"
→ "add_regime_gate on `rv(20) pct_rank < 0.8`"). Deterministic rules first; model ranking
only on capable tiers. Motivated moves only — exploration belongs in the Experiment step.

**Acceptance:** every suggestion cites the diagnostic that motivated it.
**Traces:** AGENT-011 §4, §10.

### S-5.12 Skeletons — S
`DONE` 2026-10-03 (track LANG, wave 2) — with the gate fixed, all four skeletons compile, validate and preview with non-zero entries and no warm-up violation on **8,784 real BTC-USD 1h bars (2024)**: trend 549 entries (45.7/month, warm-up 105), mean_reversion 195 (16.2/month, 73), breakout 600 (49.9/month, 20), vol_target 55 (4.6/month, 51) (`skeletons_on_btc_usd`, run with `--ignored` against the local ClickHouse; the synthetic-bar version runs by default). Entry counts are about signals only, not P&L. The breakout skeleton was rewritten to `pct_rank` because `close > donchian_upper` can never fire.

~~`crates/strategy-lang` **does not exist**.~~ **False as of 2026-09-21, and it was false when it was written under seven separate tasks.** The crate is **13,877 lines over 129 passing tests** — `lex, ast, parse, compile, decompile, diff, suggest, block, form, skeleton, catalogue, diagnostic`. What was true is worse and more specific: **nothing in the workspace depended on it.** `grep strategy-lang crates/*/Cargo.toml` matched only its own manifest. Thirteen thousand lines of the built-but-unreached shape. See the note at the head of this file.

`strategy_skeleton(hypothesis_family)` for trend, mean reversion, breakout, carry,
vol target and model signal, so a first version is never a syntax lesson.

**Acceptance:** the trend, mean-reversion and vol-target skeletons compile, validate and
preview on BTC-USD 1h with non-zero entries and no warm-up violation.
**Traces:** AGENT-011 §4; §8.2.

**Built 2026-09-21.** `crates/strategy-lang/src/skeleton.rs`: four of §4's six
families — trend, mean_reversion, breakout, vol_target — each a **compiling,
round-tripping definition rather than a text template**, because a skeleton that
does not compile is a worked example teaching a model something false.
`HypothesisFamily` is a closed enum whose body table matches it exhaustively, so
a new family cannot be added without deciding what it writes, and
`Skeleton::params()` reads the parameters off the compiled definition rather than
listing them a second time.

**`carry` and `model_signal` are refused by name**, listing the families the
engine does not have (`funding_rate`/`basis`; `prediction`/`regime`/`text` —
DATA-006 F3/F4, unbuilt). The refusal is checked against the **live registry**:
`a_refused_family_names_features_the_engine_really_does_not_have` fails the build
the day `crates/features` gains one of those names, and what it then asks for is
the skeleton, not a wider allowlist.

**The `vol_target` skeleton is not the textbook rule, and its own notes say so in
the first line.** `size vol_target 0.15` has no `SizeMode`, so it uses
`risk_per_trade`, and the volatility gate is a level on `bb_width` rather than
`pct_rank(rv(close,20), 200)` because a feature cannot take another feature as
its input. A test asserts the notes name `size_mode_not_representable` and that
the text does not claim the sizing it cannot do.

**Narrowed 2026-09-27: the `validate` half of the acceptance was failing, and for the worst
possible reason.** Three of the four skeletons emit `{{param}}` interpolation, and
`strategy-validator::validate` ran its feature-name pass on the **raw** definition, so
`ema_{{fast}}` was refused `unknown_feature`. The skeletons compiled and round-tripped, and the
very next call in the authoring path rejected them. Fixed in `crates/strategy-validator/src/lib.rs`
(see S-5.3), so "compile, validate" now holds for all four.

**What keeps it `PARTIAL`:** §8.2 asks the skeletons to "preview on BTC-USD 1h
with non-zero entries and no warm-up violation". This crate has no bars and no
`preview_signals` — they are proved to compile, validate, declare exits and round-trip; they
are not proved to fire.

### S-5.13 Rewrite the guide around the language — M
`DONE` 2026-10-03 (track LANG) — `crates/strategy-lang/docs/slv2-guide.md`, served by `get_slv2_guide(section)`: each section is at most 4 KB (asserted over heading and body), every `see` anchor the language, the validator's table and the tools emit resolves (scanned from source), the feature reference and skeletons are generated from the registry and the compiled skeletons (a test fails if the file drifts; `SLV2_GUIDE_WRITE=1` regenerates), every full strategy in it compiles, and the refusals it names are the compiler's. `slv2_guide.rs` 9/9; mutation: a padded section fails it. The legacy `authoring_guide.md` is untouched (not this track's); its "Strategy definition format" and "Expression grammar" sections should point here.

Replace the "Strategy definition format", "Expression grammar" and "Features available"
sections with one page of grammar, a worked example per hypothesis family, the units table,
the moves table, and per-family feature notes generated from the registry. JSON moves to an
appendix for the MCP client.

**Acceptance:** each SLv2 section is ≤ 4 KB (so a 7B fetches one at a time) and every `see`
anchor from S-5.3 resolves.
**Traces:** AGENT-011 §6; §8.8. AGENT-005 §3.3.

### S-5.14 F2 families — L
`DONE` 2026-10-03 (track LANG, wave 2) — registered: `pub mod families;` in `features/src/lib.rs`, five rows and four `EXAMPLES` entries in `registry.rs`, five `Kind` arms in `runtime.rs`, each its own block. `cargo test -p features` 130 pass, including `the_catalogue_and_the_constructor_agree_in_both_directions` and `every_catalogued_family_passes_the_declared_lookback_check`; the engine/registry parity test covers them; `features_f2_reconciliation` agrees with the templates' own functions to 1e-9. Registering found that `hurst` needs `n >= 257` (three block sizes need 256 returns), not 129. Side effect for Set U: the registry grew by five families, so `docs/agent/slv3-guide.md`'s generated tables are stale and `v3_guide::the_guide_tables_are_the_registries` fails until `SLV3_GUIDE_WRITE=1 cargo test -p strategy-lang --test v3_guide` is run (that file is Set U's; not touched).

Seven families, and it found two real defects on the way:

- `parkinson_vol`, `garman_klass_vol`, `rogers_satchell_vol` — agree to 1e-9, as `engine**2 == template_variance`. The engine reports a per-bar deviation (`..._vol`), the template a variance (under `variance_per_bar`); both are correctly labelled, and a second test pins that shape so a future change fails saying which claim broke.
- `rv`, `bv` — the engine sums and the template means, so they agree up to the term count, exactly.
- `autocorr` at lags 1–3 — already identical.
- `variance_ratio` — **was wrong.** See below.

**Defect 1, in the fixture.** Garman-Klass came out 7.5e-4 high. Neither implementation was at fault: `reference_dataset.py` built the intrabar path as a cumulative sum of the steps *after* the open, so `efficient_high` and `efficient_low` need not bracket the opening point, and with a 60 bps spread the observed open landed above the observed high on **2,526 bars of 6,000**. `0.5·ln(h/l)² − (2ln2−1)·ln(c/o)²` is non-negative for any bar whose range contains both ends, so the engine's per-bar floor at zero can never bind — and it was binding on fifteen bars. Fixed in the generator; the floor stays, because it is what made a silent malformation visible.

**Defect 2, in the engine.** `variance_ratio`'s registry line says "Lo-MacKinlay variance ratio at horizon q" and the implementation aggregated with `chunks_exact(q)` — every qth window, the rest discarded. That is not Lo-MacKinlay, whose contribution was the overlapping estimator. Measured at 0.4515 against the template's 0.4266 on the same 2,011 bars. Now overlapping on both sides, with the property pinned inside `crates/features` as well, since the cross-language test needs a Python interpreter.

**What keeps this `PARTIAL`**: the reconciliation can only cover families that exist twice. `yang_zhang` and `hurst` are in the templates and not in the engine; `efficiency_ratio` is in the engine and not the templates; `fracdiff` and `lookback_ensemble` are in neither. Those are gaps in the F2 set rather than drift — a family that exists once cannot disagree with itself — but the phase is not done until they exist.

Audited 2026-09-16 against the source. `crates/features` is a real engine — about 5,100 lines over 101 passing tests, with `registry.rs` (583), `runtime.rs` (1,068) and `leakage.rs` (1,301) — and `crates/backtest/src/sim.rs:987` resolves every strategy feature through `features::runtime::feature`. Proven by construction rather than by reading: `atr_14`, `parkinson_vol_20`, `rolling_std_30`, `donchian_upper_20`, `obv` and `hour_sin` all resolve and run.

Range volatility (`parkinson … yang_zhang`), realised measures (`rv bv rq jump_share`),
`hurst variance_ratio autocorr fracdiff`, `efficiency_ratio`, `lookback_ensemble` — regime
gates, vol targeting, persistence filters, and the same code the Phase 3 analysis templates
read.

**Acceptance:** the analysis templates and the strategy engine compute identical values for
the same family on the same window.
**Traces:** AGENT-011 §2 F2, P2.

### S-5.15 Intent survives, and the preview says which clause blocked — M
`PARTIAL` 2026-10-04 (S-CATALOGUE track) - Comments are now stored, beside the revision rather than on the definition (ADR-S5-14): migration `0239` `strategy_revision_notes` keyed by (user, revision hash of the canonical definition, which contains no comment), `POST /api/strategy-notes` and `GET /api/strategy-notes/{hash}` (`routes/strategy_notes.rs`). The text door of `create_strategy` stores the author's comments after a successful save, and `explain_strategy` given a stored definition reads them back by recomputing its hash. Tests: `crates/api/tests/strategy_notes_store.rs` 4/4 live PG (source order, last text wins, tenant isolation, `app_role` grants); mcp `strategy_lang` tests 18/18. Why this is not a hash risk: the stored definition is untouched, so rewording a note cannot create a revision or a trial. Still PARTIAL: `apply_move` carrying a comment forward and appending the move's reason is not wired to the store (the move works on the in-memory definition), and the route/tool round trip is unit-tested at each end, not driven over HTTP.
`PARTIAL` 2026-10-03 (track LANG) — the preview names the blocker: `preview_signals` attaches a `blocking_profile` whenever a draft never fires, measured through the preview's own evaluator (`the_preview_names_the_blocker.rs` runs the production `backtest::preview` over probe definitions: one dead clause named; clause order irrelevant; three clauses; two dead clauses give no single blocker; siblings sharing a signal measured alone; a firing entry reports the preview's own count). The tests found two real bugs in the first design (probes ignored precedence; clauses counted on bars before warm-up). Mutation-checked. The preview route now materialises parameters at their defaults (`agent_preview.rs`), so swept drafts preview for every caller. **Comments are not stored** (ADR-S5-07): they are parsed, addressed, and shown by `explain_strategy` from the author's text; they do not survive the stored form.

~~`crates/strategy-lang` **does not exist**.~~ **False as of 2026-09-21, and it was false when it was written under seven separate tasks.** The crate is **13,877 lines over 129 passing tests** — `lex, ast, parse, compile, decompile, diff, suggest, block, form, skeleton, catalogue, diagnostic`. What was true is worse and more specific: **nothing in the workspace depended on it.** `grep strategy-lang crates/*/Cargo.toml` matched only its own manifest. Thirteen thousand lines of the built-but-unreached shape. See the note at the head of this file.

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

## Decisions recorded 2026-10-03 (track LANG)

Set U landed SLv3 — a second text language over a typed graph (`strategy_lang::v3`, `/api/v3/*`,
`*_strategy_v3` tools, `docs/agent/slv3-guide.md`) — while this phase was open. SLv2 and SLv3
overlap in purpose, so before touching any task each one was decided against what Set U built:
**(a)** still needed on SLv2 as written, **(b)** already covered by Set U, **(c)** retargeted so one
piece serves both. The ADR-INDEX (`docs/ml-platform-context-pack/decisions/ADR-INDEX.md`) is outside
this track's territory and does not yet carry these rows; the orchestrator should mirror them.

| ID | Decision | Rejected alternative | Why |
|---|---|---|---|
| ADR-S5-05 | **Every validator rejection carries a worked example**, not only the third on one path. AGENT-011 §5's "a third identical rejection" is met by a stronger behaviour. | A per-run rejection counter in the MCP server, keyed by `path`. | §5 counts rejections *within a run*; `strategy-validator` is a pure function of one document and the MCP server holds no per-run state a counter could live in without becoming a second ledger. An example the model did not need costs one clause of context; a withheld example costs a model's turn. `rejects_loosening` is untouched. |
| ADR-S5-06 | **The ten tasks are decided per task against SLv3, and nothing SLv2 is removed before U-8.4.** (a) S-5.8, S-5.9, S-5.10, S-5.11, S-5.12, S-5.15 stay on SLv2/v1 — they operate on the text and the stored v1 definition, which are what the live preview, backtest and paper paths still run. (b) none closes wholly on Set U: SLv3's `validate`/`compile`/`explain`/`build` cover *v3 graphs*, not v1 definitions or SLv2 text. (c) S-5.2, S-5.13, S-5.14 are retargeted so one piece serves both: the feature registry is read by SLv2's `Catalogue`, SLv3's `v3::catalogue::features()` and `list_features` alike, so a family added to `crates/features` appears in all three; the SLv2 guide follows SLv3's `## Title` / anchor / ≤ 4 KB convention in its own file. A v3 header handed to an SLv2 tool is refused by name (`this_is_slv3`) with the pointer to the `*_strategy_v3` tools, not parsed into a diagnostic about a clause the author did not write. | Close the SLv2 tasks as superseded by SLv3. | Set U's own MASTER (§5) and phase 8 keep the old evaluators and grammars until U-8.3/U-8.4, and every strategy stored today is v1 (U-7.3 translated a corpus, not the database). Three of the ten tasks (the registry, the F2 families, the validator's teaching table) are language-independent and would have been lost with it. |
| ADR-S5-07 | **Comments are not stored on the canonical definition.** S-5.15's "the compiler stores it on the canonical node and the decompiler emits it back" is not built. They are parsed, kept on the syntax tree, addressed like every other clause (`strategy_lang::intent::notes`) and shown by `explain_strategy` when the caller supplies the text. | A comment field on `domain::strategy_def` nodes. | `domain::strategy_def::revision` documents that a comment-like field makes two behaviourally identical documents hash differently, which would make every rewording of a note a new revision and a new apparent trial for anything deduplicating on the content hash — the error `N_eff` exists to prevent. SLv3's graph excludes comments for the same reason (ADR-0045), and v1 retires at U-8.4. What would close the gap without touching the hash is a `source_text` column on `strategy_revision` (a ledger migration; reported, not built). |
| ADR-S5-08 | **The blocking profile is measured by previewing the same definition with one clause swapped in at a time** (`strategy_lang::probes`, `ProbeSet::counts`), through the platform's own preview route. 1 preview for a one-clause entry, `n + 1` for two, `2n + 1` for three or more; capped at 24 with a named refusal. | Per-clause counting inside `backtest::preview`'s loop, as the task text implies. | Set U forbids new callers of the preview's evaluation loop and retires it at U-8.3, so the new code would have been written into a loop scheduled for deletion; and the language would have to be taught to `crates/backtest`. The preview already reports how many bars a signal was true on, which is the one number every probe needs. Probes keep the strategy's declared inputs, so every probe has the same warm-up and the counts can be subtracted — asserted per run (`probes_disagree`). |
| ADR-S5-09 | **`lookback_ensemble` is registered as two families, `lookback_ensemble_zscore_n1_n2_n3` and `lookback_ensemble_returns_n1_n2_n3`**, equal-weight over exactly three lookbacks. | DATA-006's `lookback_ensemble(f, n=[…])` over any feature `f`. | The registry's names are `family_p1_p2…` over integers so a 7B can write the catalogue under a grammar, and a feature cannot take a feature as a parameter — the same limit that keeps `pct_rank(rv(close, 20), 200)` out of the language (ADR-D-14, "a scheduled language item"). An ensemble of one lookback is bit-identical to `zscore_n`/`returns_n`, tested. |
| ADR-S5-10 | **No migration `0082_feature_registry.sql`; the registry's version is the feature-set hash that already exists.** `runtime::feature_set_version_hash` covers `(feature_id, version, code_hash)` per feature, so any dataset or run that used a feature set records exactly the registry state it read. | A table persisting a registry version. | The registry is code. A table of versions is a second copy a deploy can forget to update, and nothing reads it. |
| ADR-S5-11 | **S-5.9's measurement uses eight hypothesis goals written for the test, not AGENT-009's research suite.** | Feeding `evals/tasks/planted_edge/*.yaml` prompts to the form. | Those prompts ask an agent to "find out whether it contains any exploitable structure" over a whole funnel; they carry no hypothesis for a clause fill to render, so a fill count over them would measure the harness, not the grammar. Stated so the number is not read as an AGENT-009 result. |
| ADR-S5-12 | **The skeleton and blocking tests run the production preview over a deterministic synthetic hourly series, not BTC-USD.** | Reading BTC-USD from ClickHouse in a test. | No database is available to a test. What is asserted is about the language (a skeleton fires; a clause is named as the blocker), which a deterministic series can establish; "fires on BTC-USD" is a property of the market and is not claimed. |
| ADR-S5-13 | **`suggest_moves` accepts `exit_reasons` and `regimes` from the caller** alongside the blocking profile it measures itself. | Waiting for typed producers, which would leave every rule that reads them unreachable. | Nothing in the platform yet produces labelled regime windows or per-exit-reason shares in a typed form. A caller's regime gate is a `Predicate`, checked against the registry at construction, so an unknown feature cannot be smuggled in. |
| ADR-S5-14 | **Comments are stored beside the revision in `strategy_revision_notes` (0239), keyed by the canonical definition's hash; the definition and its hash are unchanged.** Supersedes the "not stored" half of ADR-S5-07 (owner instruction: store them). | A comment field on `domain::strategy_def` (ADR-S5-07's rejected alternative: it changes the hash, so rewording a note becomes a new revision and a new apparent trial; also ~68 struct literals, mostly Set U). A `source_text` column on the revision table (a ledger migration, stores the whole text). | A side table cannot change an identity the ledger counts. Last text wins per (user, revision). |
| ADR-S5-15 | **`exit_reasons` is produced by the simulator's stored result, not transcribed by the caller; caller-supplied values still win.** | Adding an exit reason column to `Trade` (changes a type every trade reader touches). | `ExitFill.rule` already records which declared exit fired; counting it is additive in `result_document`. |

## Exit criteria

- One feature engine serves backtest, live runtime, analysis templates and the training
  frame, with no second EMA/RSI implementation and no I/O in `crates/features`.
- A strategy can be written as text, previewed without spending a trial, and explained back
  in words.
- SLv2 round-trips through the canonical JSON.
- A 7B writes a valid strategy under a grammar in ≤ 6 fills.
- The validator returns a concrete fix for every error class in AGENT-011 §5.
