# External review, checked against the code · 2026-09-14

Two documents arrived from agents that had **no access to this repository**:

- `Autonomous_Quant_Agent_Research.md` — a literature review answering `research-prompt.md`, closing with ten ranked changes.
- A conversational design memo (the "abandon 1-minute / falsifier / L0–L3 / exit-semantics" note).

Both reason from a *description* of the platform that is roughly a year stale.
It describes the v1.0 strategy JSON, the MCP surface and an overwrite-only
store, and knows nothing of the trial ledger, the sixteen-gate stack, the sealed
holdout, the null library or the synthetic venue. Read straight, they would send
us to rebuild things that exist.

This file separates three cases: **already enforced**, **wrong about this
repo**, and **a real hole we have not filled**. Every row was checked against a
file, and the file is named.

> **Citations in the source documents are unverified.** They were produced by
> models without repository access and without a browser. The papers are mostly
> well-known and plausible; `Gençay (2026), arXiv:2608.27734` is recent enough
> that nothing here should lean on it until someone reads it.

---

## 1. What they recommend that the platform already enforces

Their ten ranked changes, scored against what is merged.

| # | Their change | Status here | Where |
|---|---|---|---|
| 1 | Immutable revisions, append-before-evaluate tickets, retain failures | **Half.** Every *evaluated* artifact is immutable and hash-chained; every *authored* one is still overwritten. See §3.1. | `migrations/0043_trial_ledger.sql:115`; `crates/api/src/routes/strategies.rs:60` |
| 2 | Enforce sealed evaluation data below every access path | **Substantially done.** `get_bars` clips to the project research cutoff *and* clips `as_of` separately, so a late revision cannot leak. The sealed holdout is one-shot per lineage and the claim is logged before the read. The seed holdout is the frozen first 200 trials, platform-held. | `crates/api/src/routes/data.rs:85`; `crates/ledger/src/holdout.rs`; `crates/ledger/src/internal.rs:51` |
| 3 | Cheap causality, accounting and cost gates before broad search | **Mostly done.** Gate 2 leakage suite, Gate 3 cost sensitivity via `breakeven_multiple` at a 3× bar, Gate 4 capacity with square-root impact. *Aggregate instance limits are the missing piece* — §3.3. | `crates/backtest/src/gates/evaluators.rs:779, 904, 1035` |
| 4 | Freeze and evaluate the whole proposer/search/promotion procedure | **Recorded, not evaluated.** `policy_id`, `policy_version`, `propensity`, `exploration_flag`, `candidate_set_hash`, `prereg_hash` are REQUIRED columns on the trial. Nothing yet replays a frozen procedure end to end. | `migrations/0043_trial_ledger.sql:139` |
| 5 | Store aligned returns; expose an evidence bundle, not one Sharpe | **Done.** `trial_return_series` with a series digest, `trial_statistic`, and a sealed `Comparison` that cannot be minted without the profile, N_eff, declared effect size and both trial counts. The interval is an anytime-valid empirical-Bernstein confidence sequence. | `migrations/0043`, `0057`; `crates/backtest/src/stats/compare.rs` |
| 6 | Reevaluated portfolio archive, typed mutation, behavior dedup | **Not built.** No archive, no behavior descriptors, no complementarity scoring. §3.5. | — |
| 7 | Structured negative memory and a calibrated critic | **Deferred by decision.** `Answer<T>` carries its rung and the ledger evidence it was chosen on; every learned tier sits behind a written ledger-size trigger (ADR-P4-01). Nothing is fitted. | `crates/ledger/src/tiers.rs` |
| 8 | Budgeted multi-fidelity inner tuning | **Done, and harder than they ask.** ASHA with five non-optional mitigations, top-3 reseed with platform-held seeds before an incumbent is crowned, TPE, median early stop; searcher choice is a pure function of budget/dimensionality/parallelism, logged at `decision_tier = rule`. | `crates/research/src/search.rs` |
| 9 | Procedure-null and adversarial-evaluator suite | **Half.** `RunConfig.null_world` is part of the content hash, so a null run can never collide with its real run. The synthetic venue has seven seeded generators — three provably driftless, four with planted mechanisms carrying a `Truth` descriptor. *What is missing is running the entire loop on null worlds*, not the generators. | `crates/backtest/src/run/config.rs`; `crates/backtest/src/nulls/`; `crates/backtest/src/synthetic.rs` |
| 10 | Event-level crypto data, versioned fee tiers, depth, latency | **Not built.** Costs are a flat per-asset-class rate. §3.4. | `crates/execution/src/paper/engine.rs:101` |

**Their "two remaining experiments" are one experiment.** The memo says the only
research left is (a) market characterization and (b) a planted-edge sensitivity
test. (b) is `crates/backtest/src/synthetic.rs` — four planted generators, each
with a known mechanism and strength, and
`noise_generators_have_no_detectable_drift` keeping the null arm honest as the
code changes. What we have never done is *run the agent against it and plot the
curve*. That is a measurement, not a build.

---

## 2. Where they are wrong about this repository

Worth stating plainly, because these framed their whole priority ordering.

- **"The overwrite-only strategy store cannot reconstruct the candidate
  population behind selection-adjusted statistics."** It cannot, but the trial
  ledger can. `mlops.trial` carries `config JSONB` and `config_hash` under a
  SHA-256 hash chain with `refuse_mutation()` on UPDATE and DELETE. The
  population behind a DSR is reconstructable today. What is *not* reconstructable
  is the definition a **live instance** is running, because that still comes from
  the mutable store.

- **"Unrestricted historical-data access allows the proposer to inspect a nominal
  holdout through its own compute environment."** The read path is point-in-time
  and clipped twice — `end` bounds which bars, `as_of` bounds which revisions of
  them — and both respect the project horizon. The sealed holdout claims the
  lineage *before* any data is read, so an abandoned claim still burns the
  lineage. Their rank #2 is largely spent effort here.

- **"No version history."** True of `strategy_definitions`. False of anything
  that was evaluated.

The pattern: they correctly identify what a system like this needs, and
misattribute the gaps, because they were looking at the authoring surface — the
one part of the platform the context pack never covered.

---

## 3. Real holes, confirmed in the code

### 3.1 The authoring store is the last mutable thing in the evidence path

`crates/api/src/routes/strategies.rs:60` is still
`INSERT ... ON CONFLICT (strategy_id) DO UPDATE`. Two consequences:

- A live instance's behaviour is not reconstructable from any immutable record.
- `RunConfig.strategy_version` is a hash of a document that can be replaced under
  it, so two runs sharing a `strategy_version` are only equal if nobody re-saved
  in between.

The ledger already proves the pattern works. This is the cheapest high-value
change on their list and the only one of the ten that is genuinely rank-1.

### 3.2 Exit semantics — the memo's sharpest point, and it is correct

`crates/domain/src/strategy_def/actions.rs:55`:

```rust
pub enum ActionKind {
    PlaceOrder { order: OrderSpec },
}
```

One variant. No stop, no target, no time-based exit, no trailing stop. The
expression grammar is stateless — every expression is a comparison over the
current bar — so none of the path-dependent exits is expressible either:

| Exit | Needs | Expressible today |
|---|---|---|
| Signal reversal | current features | yes, as another OR'd signal |
| Time-based | bars since entry | no |
| Stop-loss / take-profit | entry price | no |
| Trailing stop | running max since entry | no |

`risk_overrides` **is** part of the definition struct
(`crates/domain/src/strategy_def/mod.rs:69`), so the hashing hole the memo
predicts does not exist — position limits are inside the content hash. But
`max_position` is a limit, not an exit. So case (b) of their dilemma holds: the
risk layer is silently doing the exit logic, and every strategy the agent can
write is entry-only. The builder dropping exit rules with a warning is the tell.

This is the largest single constraint on what the agent can discover, and no ADR
covers it, because the context pack never touched the strategy language.

### 3.3 Capacity is measured per run, not per definition

Gate 4 takes `adv_usd`, `daily_vol`, `peak_daily_notional` and a `sharpe_by_aum`
curve — all from one run's bars and trade list. Nothing aggregates live notional
across the many instances of one definition. Fifty instances of a definition are
one position at 50× size with correlation 1.0, each of whose backtests assumed it
traded alone. The risk layer currently sees diversification that does not exist.

### 3.4 The cost model is flat where the whole question is the tier

`crates/execution/src/paper/engine.rs:101` — `fee_rate: dec!(0.0025)`, one
number per asset class. No maker/taker split, no tier, no rebate, and nothing
versions it. Gate 3 asks "how wrong can the cost model be before the edge dies",
which is the right question, asked against a cost model that cannot express the
axis that matters most.

The memo's break-even-horizon algebra is the part of these documents most worth
keeping:

```
t* ≈ ( c / (k · IC · σ₁) )²
```

with `c` the round-trip cost, `k` a selectivity factor (~1.5), `σ₁` the
one-minute vol. Two consequences follow that are policy, not analysis:

1. **Cost and signal quality enter symmetrically and quadratically.** Halving the
   fee tier is worth exactly as much as doubling the information coefficient. One
   is a conversation with an exchange; the other is a research programme that may
   fail.
2. **There is a horizon below which nothing is searchable**, and it is computable
   per pair per regime from data we already store. That frontier belongs in a
   gate — a candidate whose holding period sits below it should be refused before
   it consumes a trial, not after.

### 3.5 Nothing searches for diversity, and nothing measures novelty

No archive, no behavior descriptors, no complementarity scoring, no bandit over
research directions. And the memo's proposal-bias argument is one the pack never
made: the LLM's prior over "trading strategy" is the *published* corpus, which
McLean & Pontiff's decay result makes adversarially selected against us. Nothing
measures a candidate's distance from published work, so an obvious rediscovery
costs the same trial budget as a novel hypothesis.

### 3.6 USDT is treated as USD

`crates/api/src/routes/portfolio.rs:47` —
`matches!(currency, "USD" | "USDC" | "USDT")` — and `rollup/paper.rs:41`
converts stablecoins 1:1. A depeg therefore reads as alpha, and any cross-pair
discovery contains an unmeasured fraction of stablecoin drift. Small fix, real
defect, and exactly the kind of thing the platform is otherwise built to refuse.

---

## 4. The ideas worth adopting that the pack does not contain

Not in SPEC.md, not in any ADR, not implied by anything merged.

**The oracle test as a permanent member of the adversarial suite.** Run a
perfect-foresight strategy — one that knows the next bar's close — through the
backtester at the real cost model. At one-minute bars it must *lose money*: with
σ ≈ 7bp, `E|Δp| = σ√(2/π) ≈ 5.6bp` against a 20bp round trip. If an oracle is
profitable at one minute in our simulator, **the cost model is broken**. That
makes it a simulator-validation test and a horizon-feasibility test in one call.
It is an afternoon's work and it should run on every cost-model change. Highest
value per hour in either document.

**Invert the agent's job: falsifier before discoverer.** The argument is reward
density, not philosophy. "Find alpha" yields a usable learning signal perhaps
twice a year; "prove this is noise" yields one every few minutes and is usually
right. The degenerate policy — kill everything — is blocked by defining what
counts as a kill: a specific, reproducible, falsifiable defect, checkable by a
human in ten minutes, stored with its reproduction. *"P&L is 94% in six bars
during a window where OHLCV shows a 40% wick with no supporting prints"* counts.
*"Sharpe is only 0.4"* does not. Kills that cannot be reproduced score zero.

This fits the pack's grain exactly. The `array_length` NULL-CHECK hole is
precisely the class of defect a kill-rewarded agent enumerates, and only a live
database found it. It also implies a result type we do not have —
**`report_evaluator_defect(reproduction)`, scored above a promotion** — because
an agent scored on promotions that discovers the simulator lets it buy the
intrabar low will farm it silently, since reporting it destroys its best result.

**The L0–L3 decomposition, and the argument for not unfreezing the grammar.**
The grammar cannot express conjunction — but `model_forecast → threshold` is a
conjunction learner, and `NodeKind::ModelForecast`, `Inference`, `Decision`,
`Combine` and `Sizing` all already exist
(`crates/domain/src/strategy_def/nodes.rs:163`). Split the hypothesis space:

| Level | What varies | Search method | Lineage today |
|---|---|---|---|
| L0 target | what is predicted, at what horizon | human/agent hypothesis | **none** |
| L1 estimator | features, model class, training protocol | purged CV, regularization | model ledger, versioned |
| L2 policy | thresholds, sizing, exits — the JSON graph | typed structural search | **overwrite-only** |
| L3 parameters | numeric bindings | TPE / ASHA | `RunConfig.params`, hashed |

The observation that makes this cheap: **our model infrastructure has better
provenance than our strategy infrastructure.** Moving the expensive search from
L2 into L1 relocates it into the part of the system that already has versioning,
`TrialTicket` and immutability. The condition graph becomes a thin policy
wrapper and the frozen grammar stops being a limitation. The mechanism — the
thing pre-registration is supposed to pin — was never in the comparison
operator; it is in the feature family and the target definition.

**Self-improvement must target process, not profit.** The arithmetic:

| | Volume | Latency | Label quality |
|---|---|---|---|
| Alpha feedback | ~24 / year | ~90 days | noisy |
| Process feedback | thousands / year | seconds | unambiguous |

Two orders of magnitude apart with opposite latency profiles. Twenty-four noisy
labels a year on a 90-day lag is not a small dataset; it is not a dataset. So
proposal validity, cost-awareness, novelty, compute estimation and calibration on
*process* predictions are learnable, and "will this make money" is not. The
conclusion is uncomfortable and correct: the agent will get rapidly better at
proposing well-formed, novel, cost-aware, causally valid candidates and only
glacially better at proposing profitable ones — and that is most of the value,
because the statistical machinery then stops spending its budget on impossible
candidates. It is also a direct argument against ever fine-tuning the proposer on
backtest returns, which would move the overfitting into model weights where no
per-strategy statistic can see it.

**Reframe what the agent is for.** Not "invent new signal families" — the
honest proposition is: *for pair X, in regime Y, at horizon Z, under fee tier F,
which known family works, with what parameters, and when does it stop working?*
Nobody publishes that cross-product. It is enormous, genuinely valuable, and
tractable in a way "discover novel alpha" is not.

---

## 5. What follows

Ordered by evidence per hour, not by size.

1. **The oracle test.** An afternoon. May invalidate the entire target horizon,
   and validates the simulator either way.
2. **Exit semantics.** Decide whether `ActionKind` grows or the risk layer is
   declared to be the exit logic. Both are defensible; the current silence is
   not. Needs an ADR before any code.
3. **Immutable strategy revisions.** Retire the upsert; `strategy_id` becomes a
   pointer.
4. **Fee tiers in the cost model, then the tradeable-frontier gate.** Compute
   `E|Δp(t)|` vs `2c` per pair per regime and refuse to search below it.
5. **`report_evaluator_defect` as a scored result type**, and kill-reward for the
   agent.
6. **Aggregate capacity per definition**, not per instance.
7. **Numeraire.** Stop treating USDT as USD.
8. **Run the loop on the null worlds and the planted generators.** The
   instruments exist; the ROC curve for our discovery procedure does not.

Items 1, 7 and 8 need no design decisions. Items 2, 3 and 4 each need an ADR
first — they change what a strategy *is*, and the pack's rule is that a judgment
call gets written down before it gets built.
