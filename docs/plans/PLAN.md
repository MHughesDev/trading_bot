# PLAN — the one map to the finish line

**Read this first. Update it every time something closes.**

This is the operational file: where the programme actually is, what is left, and
in what order. [ROADMAP.md](ROADMAP.md) is the *reasoning* — why the stages are
ordered by the evidence clock, what "done" means, which gates need a human. Come
here for what to do; go there for why it is done in that order.

**Why this file exists at all.** ROADMAP §2 was written on 2026-09-15 and said
"Set S — 0/189, NOT STARTED" six days after Set S started. Not because anybody
was careless: because the counts were **restated by hand** in a file nobody
recomputed. So the counts here are *derived* from the plan files, and
`crates/invariants/tests/the_plan_counts_what_the_plans_say.rs` fails the build
when this file and the plan files disagree. A map that can go stale silently is
worse than no map, because people steer by it.

---

## 1. Where we are — **47% left**

Derived 2026-10-01 from the `### <TASK-ID>` headings and their status markers
across sets P, S, T and U. Size-weighted **S = 1, M = 2, L = 4**: 189 tasks of
wildly different sizes averaged flat would flatter us.

| counting rule | % of the work left |
|---|---|
| strict — only `DONE` counts | 65% |
| **`PARTIAL` counts half** | **47%** |
| generous — `PARTIAL` counts done | 29% |

Take the middle number and treat it as **±10**, not as a measurement. `PARTIAL`
is 239 of 668 units and in this repository it spans "complete, one database test
unwritten" to "the types exist and nothing calls them". The spread between the
strict and generous rows *is* the uncertainty, and it is honest to show it rather
than pick one and present it as precision.

**The headline moved from 35% to 49% on 2026-10-01 because the plan grew, not because work
regressed.** Set U (137 units) was added after an audit found that strategies are evaluated by three
different programs, that indicators are never computed on paper, and that no strategy can call a
model in any mode — work no previous set had planned. See [Set U](plan-sets/set-U/MASTER.md).

### By set

| set | subject | tasks | units | done | partial | % left |
|---|---|---:|---:|---:|---:|---:|
| **S** | The Agent Platform | 189 | 446 | 208 | 217 | **29%** |
| **P** | Data Reach | 30 | 51 | 14 | 16 | **57%** |
| **T** | Forward Evidence | 17 | 34 | 2 | 6 | **85%** |
| **U** | Unified Strategy Engine | 53 | 137 | 10 | 0 | **93%** |

P and T are 16% of the weight and almost entirely ahead of us — and they are the
two that carry a **calendar**, which is why §3 puts them first despite Set S
being where the code is.

**Both were audited for the first time on 2026-09-21 and both figures moved, in
opposite directions.** Set P is better than published (84% → 79%): four tasks
were further along than their markers said, including a 749-line census with its
acceptance as named tests and a published nine-series result. **Set T is worse
(82% → 88%), and it is 1 of 17 rather than 3.** Two of its three `DONE` markers
do not survive contact with the tree — T-0.1's wire is not in place (the
observer is constructed only in tests, and `ExecutionEngine::with_fill_observer`,
the method this phase file instructs a track to call, **does not exist anywhere
in the workspace**), and T-0.3's own note says *"This criterion is not met"*
above a `DONE`. The one task that survives does so because its author wrote its
partial down honestly.

**The consequence was not the percentage, and it is now discharged.** T-0.1 meant
a credential alone would not start the evidence clock: with no production call site
for the fill observer, `mlops.forward_observation` stayed empty and Gate 15 stayed
`NotComputable`. That second blocker was in no plan file and would have surfaced
only after H-1 was spent.

**Closed 2026-09-27.** `apps/platform/src/main.rs` installs
`reconciliation::recorder::ForwardRecorders` before anything can produce a fill, so
the process is on the fill stream and two guards keep the call ahead of the paper
engine, the execution engine and the server. `ExecutionEngine::with_fill_observer`
was **not** created: it never existed because the per-engine field was replaced by
the global registry, so this file's instruction to call it is stale rather than
unmet. What remains needs a deployment — `ForwardRecorder` is per-deployment and
takes a `DeploymentSpec` plus an opening balance — so the registry's verdict is
`NoDeployment`, which is a different fact from `NotInstalled` and is logged at
startup so an operator can tell which. **H-1 would now produce a
`forward_observation` row.**

### Set S by phase

| phase | tasks | units | done | partial | not started | blocked | % left |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1 — Contracts | 20 | 34 | 29 | 5 | 0 | 0 | 7% |
| 9 — Context as retrieval | 20 | 56 | 46 | 10 | 0 | 0 | 9% |
| 8 — Integration and honesty | 9 | 24 | 18 | 6 | 0 | 0 | 12% |
| **4 — Memory and evaluation** | 17 | 39 | 18 | 19 | 0 | 2 | **29%** |
| **3 — The catalogue** | 19 | 50 | 20 | 28 | 2 | **0** | **32%** |
| **6 — The Research Desk** | 24 | **66** | 22 | 44 | 0 | 0 | **33%** |
| 7 — The monitor | 13 | 29 | 10 | 18 | 0 | 1 | 34% |
| 0 — Ground rules | 6 | 7 | 2 | 5 | 0 | 0 | 36% |
| 10 — Modes and the lifecycle | 16 | 41 | 9 | 32 | 0 | 0 | 39% |
| 2 — One runtime | 16 | 32 | 9 | 19 | 4 | 0 | 42% |
| 5 — The language | 15 | 36 | 12 | 15 | 9 | 0 | 46% |
| **11 — Continuous research** | 14 | 32 | 0 | 28 | 4 | 0 | **56%** |

**Phase 3 moved 42% → 32% and its blocked column reached zero**, which is the
larger fact: all four `BLOCKED:UPSTREAM` markers in Set S are gone. Three were
closed by building the thing they waited on (a perturbation runtime mode, a
return-series route, a `by_regime` producer) and one — S-3.17 — turned out to have
had a shipped tool sitting behind the blocked marker the whole time. **Phase 10
moved the other way**, 37% → 39%, because S-10.14's `DONE` did not survive reading
its third acceptance clause.

Phase 4 moved 45% → 29% on 2026-09-23: a second audit against acceptance
criteria (not code presence) found four `PARTIAL` markers under-claimed
(S-4.3, S-4.5, S-4.7, S-4.9) and four over-claimed, where the note asserted
something false about the tree rather than merely an old date — see the phase
file's 2026-09-23 addendum. Phases 0, 3 and 11 also moved since this table was
last derived (2026-09-21) from prior sessions' landings that were never copied
in here; this pass re-derives all of Set S, not just phase 4.

The block that matters by *volume* is still **6 — the Research Desk (66
units)**, but it has now moved twice **without anybody writing a feature**:
47% → 36% on 2026-09-21 (five tasks done and marked `PARTIAL`, one marked
`DONE` that failed its own acceptance by 28 files), then 36% → 33% on
2026-09-23 (two more under-claimed, four notes corrected without a marker
change, including one — S-6.24 — where a fix had already landed in this same
file's own prose without the per-task block being updated to match, a repeat
of a mistake the first audit had already caught once for other tasks). The
moves keep going both ways, which is the point — an audit that only promotes
is a press release.

Phase 3's ten blocked units are new and are not a setback: four tasks moved
`NOT STARTED` → `BLOCKED:UPSTREAM` once somebody checked *why* they were
unstarted. Two ClickHouse schema files were never applied on this deployment,
and that single fact kills three tools. Blocked and not-started weigh the same
in the percentage and mean completely different things to whoever picks the work
up.

Phase 11 still has the highest percentage and the smallest body.

### Set P and Set T by phase

| set | phase | tasks | units | % left |
|---|---|---:|---:|---:|
| P | 0 — Measure, start the irreversible clock | 6 | 10 | **100%** |
| P | 1 — Deep backfill | 7 | 13 | **100%** |
| P | 2 — Quality and eligibility | 6 | 10 | 20% |
| P | 3 — Alternative data | 6 | 9 | 100% |
| P | 4 — Derived access and the source fence | 5 | 9 | 100% |
| T | 0 — Reconciliation and the first observation | 4 | 8 | 25% |
| T | 1 — The deployment path | 5 | 10 | **100%** |
| T | 2 — The paper fleet | 4 | 8 | 100% |
| T | 3 — Ramp mechanics | 4 | 8 | 100% |

### The older sets, and what is genuinely left in them

Sets A–L use a different format (one `Completion:` line per MASTER, no per-task
markers) so they are not in the 46%. Most are closed; **two are not**, and saying
"A–L are done" would be wrong:

| set | state | what remains |
|---|---|---|
| A, B | superseded / complete | nothing |
| C | **CLOSED**, 54/68 shipped | 2 items worth doing (`ws/live.rs` per-subscriber clone), 1 nobody filed (`hot_path.rs` builds a `json!` per trade tick inside the allocation-free pipeline) |
| D | 31/33 (94%) | 2 deferred, recorded |
| **E** | **7/30 (23%)**, 6 partial, 16 open | production readiness. **10 items block unattended paper trading and 4 block a paper fill at all** — those four are Stage 0 work, not Stage 3 |
| F, G, I, J, K | closed / complete / superseded | F has 2 deliberately reversed; G has 4 hygiene items |
| **H** | **47/64 (73%)** | AI Model Studio phases 4–5, folded into the reserved Set N |
| L | phases 0,1,2,5,6 complete | 3–4 built but **never run live** — the same blocker as S-4.14 |

---

## 2. What "done" means

Six conditions, from [ROADMAP §3](ROADMAP.md). Not "every set complete" — the
sets are a means.

| # | condition | measured by |
|---|---|---|
| DONE-1 | A question typed into the desk produces a sealed pre-registration, a platform-computed verdict and a knowledge unit with no human step between, and the run replays byte-identically | AGENT-013 §13, `lifecycle.reproduce` |
| DONE-2 | `cost_per_knowledge_unit` measured per run, 90-day trend flat or falling while the knowledge base grows | AGENT-013 §10 |
| DONE-3 | ≥ 1 strategy holds a `paper_v1` pass whose **forward fills reconcile** with its backtest distribution, **and** ≥ 10 rejected with a named failure class a later run retrieved and did not re-test | Gate 15 + AGENT-007 negatives |
| DONE-4 | The `judgement` suite scores at or above threshold on frontier; every local tier reports a measured score or `unmeasured` | AGENT-009 §3 |
| DONE-5 | No trial anywhere without a ledger row and a sealed pre-registration, through **every** door including the external MCP server and the UI | INV-16 + D-S12 + AT-64 |
| DONE-6 | A continuous session runs a week unattended inside budget and returns a digest a human acts on, having chosen its own agenda and stopped itself when it stalled | AGENT-014 §12 |

**DONE-3 is the only one with a calendar in it.** Forward fills accumulate at one
day per day and no amount of engineering compresses that. Everything in §3's
order follows from this one fact.

---

## 3. The route, in order

### Priority 0 — things that start a clock (do these before anything else)

Nothing here is large. All of it is calendar-bound, which means a week not spent
on it is a week the finish line moves.

| # | what | where | state |
|---|---|---|---|
| 0.1 | **Deploy one hand-authored strategy to paper and start the clock** | T-0.4 | `BLOCKED:HUMAN` — see §5 |
| 0.2 | The four Set E items that block a paper fill at all | Set E, Stage 0 subset | open |
| 0.3 | Coverage census — what history actually exists | P-0.1, P-0.2 | not started |
| 0.4 | Funding rates and open interest, forward | P-0.3 | not started |
| 0.5 | Deep backfill: Coinbase, Kraken, Alpaca | P-1.1 … P-1.5 | not started |

Set P phase 0 is the "irreversible clock" phase by its own title: data not
collected today cannot be collected later at any price. It is 100% remaining.

### Priority 1 — Set S, the agent platform

Ordered by *what unblocks what*, not by phase number.

| # | what | phase | units left | why here |
|---|---|---|---:|---|
| 1.1 | **The strategy language** — `crates/strategy-lang` does not exist; S-5.5…5.9, 5.12, 5.13, 5.15 | 5 | ~24 | It is the capability ceiling. `crates/features` computes ~50 families and the simulator resolves them, but the agent cannot *express* a strategy beyond what the language allows. Everything downstream inherits that ceiling |
| 1.2 | **The Research Desk** remainder | 6 | ~31 | The largest single block, and it is where every other phase becomes visible to a human. 62 of 24 tasks are `PARTIAL` — this is finishing, not starting |
| 1.3 | Memory and evaluation | 4 | ~12 | Feeds DONE-2 and DONE-4 |
| 1.4 | The catalogue remainder | 3 | ~22 | 12 tasks not started; includes tools other phases assume |
| 1.5 | Modes and the lifecycle | 10 | ~15 | 30 units partial; mostly finishing |
| 1.6 | One runtime | 2 | ~14 | Includes deleting `driver.rs` (S-2.10) — one of two loops is still there |
| 1.7 | The monitor | 7 | ~10 | Gates Stage 3 |
| 1.8 | **Continuous research** | 11 | ~28 | Highest % left, smallest body. Directly DONE-6 |
| 1.9 | Ground rules remainder | 0 | ~4 | Bookkeeping, one human gate |

### Priority 2 — and after

Stages 2–6 as [ROADMAP §4](ROADMAP.md) has them: complete the evidence surface
(16/16 gates judging on real numbers), paper at scale (Set T phases 1–3, Set E,
COMP-007 live), capability depth (Set N absorbing H 4–5), accumulated knowledge,
then the human decision gate at capital.

---

## 3a. Running agents in parallel — **in one worktree**

The [Execution Board](EXECUTION-BOARD.md) was written for agents on separate
worktrees, where the hazard is the merge. In **one** worktree there is no merge
and the hazard is the opposite: two agents editing the same file, and one agent's
half-written file breaking another's build.

So the partition is by **file territory**, and it has three rules.

### Rule 1 — every track owns a disjoint tree, and touches nothing outside it

| track | owns | builds with |
|---|---|---|
| **A — the desk** | `frontend/**` | `npx tsc -b --noEmit`, `npx vitest run` |
| **B — the language** | `crates/strategy-lang/**` | `cargo test -p strategy-lang -j 2` |
| **C — the session** | `crates/research-session/**` | `cargo test -p research-session -j 2` |

Chosen so the trees are disjoint *and* the builds barely interact: track A runs
no cargo at all, and B and C are leaf crates that nothing else in the workspace
depends on yet. A broken file in one cannot fail the other's test run, which is
the failure mode that makes same-worktree parallelism miserable.

### Rule 2 — the shared files belong to the orchestrator, not to any track

Nobody but the integrating session touches these, and a track that needs one
**stops and reports the one line it needs** rather than editing it:

`Cargo.toml` · `Cargo.lock` · `apps/platform/src/main.rs` ·
`crates/api/src/lib.rs` · `crates/api/src/routes/mod.rs` ·
`crates/mcp-server/src/taxonomy.rs` · `crates/mcp-server/src/spec.rs` ·
`migrations/**` · `frontend/src/App.tsx` · every file under `docs/plans/` ·
`docs/specs/README.MD` (the user's — never stage it)

Migration numbers are pre-allocated when a track needs one, before it starts.

### Rule 3 — tracks do not commit, and do not run the workspace suite

One index, several writers, is a race nobody needs. Tracks leave their work in
the tree and report; the orchestrator reads, verifies and commits.

`cargo test --workspace` from a track would serialise every other track behind
it on one `target/` and one cargo lock. **`-p <your crate>` only.** The single
workspace run happens once, at integration. `.cargo/config.toml` pins `jobs = 2`
and raising it OOMs the page file — this is not negotiable and is not about
politeness.

### What each track is for

- **A** finishes phase 6. 22 of its 24 tasks are `PARTIAL`, so it is finishing
  surfaces that already render, not starting them. It must **not** add a call to
  a route that does not exist — `every_client_path_is_served` fails the build for
  it, and correctly.
- **B** builds phase 5's missing half. `crates/strategy-lang` exists as an empty
  registered crate; everything in it is new code, which is why it is the safest
  thing to parallelise.
- **C** builds phase 11's pure halves — the stall detector (§7.1) and the digest
  (§2.4). Both are functions over fixtures with no database in them, which is the
  same reason `charter.rs`, `agenda.rs`, `fold.rs` and `select.rs` are testable.

---

## 4. How the work is done

These are not preferences. Each one is here because breaking it cost something.

**Authority order.** `INVARIANTS.md` > `spec/SPEC.md` > `reference/*` > own
judgement. If the spec looks wrong, write it into `OPEN-QUESTIONS.md` and
implement neither version. Every judgement call the spec does not cover gets an
`ADR-INDEX.md` row with the rejected alternative and the reason.

**REQUIRED / no default / never / enforced by** → a schema constraint, a GRANT, a
type, or a CI test. **Never a comment.** Where a rule must exist in two places
(Rust and SQL), a test holds the two equal cell by cell.

**Never** add a non-PIT read path, a way to run a trial without a ledger row, or
a flag that disables either. That applies in tests too.

**Never report a partially enforced invariant as enforced.** A `PARTIAL` that
names what it does not cover is worth more than a `DONE` that does not.

**Verify at phase end, not per task.** Build the whole phase, then run
`cargo test --workspace -j 2` once. Full parallelism OOMs the page file.

**The dominant defect shape in this repository is "built but unreached"** —
complete, tested code with no caller. Four variants seen so far:
1. no caller at all;
2. a caller with no callee across HTTP (the frontend calling a route nothing serves);
3. a reader of a field nothing writes;
4. **the gap list itself** — of five tools once declared "owed", none needed building.

Before building anything downstream, check whether the upstream thing already has
a caller. Before declaring a gap, check whether it is really a gap.

**Database work is verifiable locally.** A native PostgreSQL 17.7 answers on
`127.0.0.1:5433` as `postgres`/`postgres`. Port 5432 is closed, which is why the
repo default looks absent. pgvector is not installed, so the full chain cannot
run locally — a test about one table applies its migration directly.

---

## 5. Blocked on a human — the short list

These cannot be unblocked by writing code. They are in rough order of what they
cost while they wait.

| # | what is needed | what it unblocks | tracked as |
|---|---|---|---|
| H-1 | **Deploy one hand-authored strategy to paper** | The evidence clock. DONE-3 is the only condition with a calendar in it, and this is the day it starts. **Its engineering prerequisite is discharged as of 2026-09-27**: reconciliation is now installed on the fill stream at startup, so a deployment will actually produce `forward_observation` rows. Until 2026-09-27 this gate would have been spent and produced nothing | T-0.4 |
| H-2 | **A frontier provider credential** | No live agent session has ever run. Two Set S acceptance criteria sit behind it, and Set L phases 3–4 are "built, never run live" for the same reason | S-4.14, Set L §4 |
| ~~H-3~~ | ~~Sort the working tree~~ | **Discharged 2026-09-21.** The ~20-file tree the triage describes is gone — every file has been landed in a commit. `git status --short` now shows one file that is not an agent's in-flight work, and it is the one the user has confirmed is theirs. No decision was needed in the end | S-0.1 |
| H-4 | Decide the capital gate | Stage 6 | ROADMAP §8 |

**H-1 and H-2 are the two that matter**, and H-4 is the third. Everything else on
this page is work I can do; those are work only you can do, and each gates a
condition that no amount of building satisfies.

**What is deliberately *not* on this list any more.** On 2026-09-21 the user asked
for the open engineering questions to be **answered rather than carried**, and all
26 were — 19 standing in `OPEN-QUESTIONS.md` and 7 raised that day — recorded as
**ADR-D-01 … ADR-D-26** with the alternative considered and the reasoning. The
mechanism is the one already used for ADR-P2-24…27 and OQ-05/OQ-08: delegated,
decided, written down. A question carried is a question somebody re-derives, and
this programme had accumulated a year's worth of them in six days. The three that
changed behaviour in a safety direction are ADR-D-11 (a hard-stale feed halts
rather than scaling, because a quarter-size position priced off a dead feed is the
same error at 25% of the size), ADR-D-05 (there will be no
`preregistration_required` boolean — a field whose only purpose is to be set to
the unsafe value should not exist) and ADR-D-03 (`ask_above_usd` fires on an
unverified price, and *only* that gate does, because it can pause and cannot
spend).

---

## 6. Progress log

Newest first. One line per closing, with the evidence.

| date | what | evidence |
|---|---|---|
| 2026-10-01 | **Set U Phase 0: the contracts every later phase speaks.** Values where absence is a value (`missing(reason)`) and a number cannot be `NaN`; Kleene three-valued logic defined once; the canonical `tbot.strategy/3` graph with its sink typed per kind, so a trading graph with a selection sink is unrepresentable; trade intents covering the terminal; selections that account for every universe member, selected or excluded with a reason; deployments that cannot be confused with backtests; typed model contracts with derived contracts for every existing kind and three new ones (`classifier`, `regressor`, `transform`); and a 74-operator registry that the checker, palette, text language and agent catalogue will all be generated from. One acceptance line was corrected rather than the code: a forecaster's derived `direction` is a label (`cat`), not `probs`, because today's server returns a label and deriving `probs` would promise what the serving path does not deliver. FEAT-007's examples were aligned with the format as built (a bare string is always a reference; literals are `{"str": …}`; no expression strings in the canonical form). | domain 217 lib tests (41 new in `strategy_graph`, 8 in `contract`), clippy clean on the new files, `cargo check --workspace --all-targets` clean; Set U 100% → 93%; headline 49% → 47% |
| 2026-10-01 | **The strategy system was audited end to end, and redesigned.** The audit traced every path a strategy takes and found ten verified defects, of which four mean the platform cannot do what it appears to: **three different programs evaluate a strategy** (the backtester's own loop over a stateless interpreter, a third copy of that loop for the agent's preview, and `StrategyInstance` on paper), and they have drifted; **on paper, indicators are never computed** — the hot path emits only bars, nothing in production emits feature values, and the slots start at `NaN`, so an indicator strategy never trades (the one passing paper test injects indicator values by hand); **no strategy can call a model in any mode** — the backtester has no inference and `set_inference_outputs` has zero callers; and **every model answers as a forecast**, with missing inputs filled with `0.0`. Also: strategies can only place market orders, selection logic has no caller, backtests skip the risk gate, the grammar is implemented three or four times, and the backtest venue facade is market-only and reports no fills. The redesign, on the owner's direction: one pure deterministic engine every environment drives; typed dataflow graphs of two kinds (trading, selection); trade intents covering the terminal, planned into orders by one shared planner; decisions on bar close, own fills and schedules, never per tick; typed model contracts resolved point-in-time; a native simulated venue on the paper venue's core (**reversing the owner's 2026-06-08 choice of market_simulator as the default backtest venue**, flagged); the risk gate in every environment; and a nightly replay that proves paper and backtest still decide identically. | [FEAT-007](../specs/FEAT-007-unified-strategy-engine.md); ADR-0039…0047; [Set U](plan-sets/set-U/MASTER.md), 53 tasks, 137 units; headline 35% → 49% because the plan grew |
| 2026-09-28 | **A `backfill` job had no worker, and the two obvious ways to give it one are both silent.** `apps/platform` registered GateAdvance, DataQc, SimulatePaths, Campaign, ResearchRun and Session and nothing for `backfill`, so every submitted window queued for ever — and `ReqwestGet`, the production `HttpGet`, had zero construction sites anywhere in the workspace, which is the same finding from the other end. Registering one worker per venue does not work, because `WorkerPool::register` keys on `JobKind` and all three adapters answer `Backfill`, so two of the three vanish while the pool still reports itself as having a backfill worker. Registering one worker over one adapter is worse: **`BackfillEngine` never compared the manifest's source with its own**, and `BarStoreSink` passes `manifest.source` to `write_collected` to select the declared vendor lag, so a window claiming `alpaca_rest` served by the Coinbase adapter would have run to completion writing candles stamped 900 s late — or, the other way round, genuinely delayed bars stamped at 60 s, which makes inadmissible rows look admissible with nothing in them to say so afterwards. Hence a `BackfillRouter` keyed on each adapter's own `source_name()`, and a guard that refuses a mismatch **before** `state.ensure` so no cursor row is left pointing an operator at a window that is fine. Alpaca is deliberately left unregistered rather than half-wired: `HttpGet` sends no authorization header, so a window naming it gets a terminal `no_backfill_engine` listing the routable sources instead of a venue 403 that reads like a plan restriction. **The free-only fence became a mechanism**: migration 0104 adds `cost_class NOT NULL` with no `DEFAULT` (the absence is the mechanism — a `DEFAULT 'free'` would undo the crate-side refusal), and `cargo xtask check-free-sources` *calls* `dataplane::source::check_declared_cost_classes` rather than restating the predicate. It failed on its first run against two sources 0098 added that 0104's first draft had not classified, which is exactly what it is for; and its own first statement-splitter was `find(';')`, which ended the 0046 insert at the semicolon inside row 1's note and reported `OK — 4 source(s)` over a table of nineteen — a guard checking a quarter of its subject while printing a number that reads like a measurement. **Eight Set P markers were understating**: P-0.5, P-1.3, P-1.4, P-1.6, P-4.1, P-4.2, P-4.3 and P-4.5 all had acceptance clauses with named tests behind `NOT STARTED`. Also fixed: `crates/ledger/tests/pg_gate_evidence.rs` did not compile on main (E0689), unnoticed because nothing had run `-p ledger --all-targets`. | jobs 103 across 5 targets, platform 18, xtask 19, dataplane 305, collectors 232, all green; clippy `-D warnings` clean; 0104 written LF-only per `.gitattributes`; Set P 79% → 57%, 9/30 done; phase 1 0% → 43%, phase 4 0% → 20%; headline 37% → 35% |
| 2026-09-27 | **Two authoring surfaces were refusing the thing they exist to accept.** In `crates/strategy-validator`, `validate()` ran its feature-name pass on the **raw** definition while every other pass read the parameter-substituted copy, so `resolve_feature("ema_{{fast}}")` was `None` and **every definition with a swept indicator period was refused `unknown_feature`** — `{{param}}` interpolation is the whole of v1.2, `compile.rs` emits it for any `input x = ema(close, fast)`, three of the four worked skeletons use it, and the persist route runs `validate()` first. So a hard-coded-period strategy validated and the same strategy made sweepable did not, with a message that offered no repair (`ema_{{fast}}` is twelve characters against `ema_20`'s six and the edit-distance cap is three). On the desk, **S-6.22's 375px clause was not "unverifiable" as the plan file said — it was false, twice**: `Hide panel` set `hidden` and nothing else, and `.desk-side { display: flex }` outranks the UA's `[hidden] { display: none }` on *origin*, leaving a `position: fixed` aside over 345 of 375 pixels with its own close button underneath; and `.desk-rail-toggle` is `position: absolute` under 768px with nothing in its chain positioned, so the only control that dismisses the rail sheet on a phone landed under `.nav` at the top-left of the viewport. Both drawers also *opened* by default at phone width. **Four more closed on the way**: `esc` called `deskApi.cancel` directly, skipping its own confirmation and firing from inside the composer and from behind any modal; `conflictPairs` dropped a conflict whose partner was in another bucket, so `Conflicts (0)` and "Nothing in this bucket disagrees" rendered over data saying otherwise; `EvidenceChip`'s `onOpen?.(id)` gave four live call sites a pointer, a focus ring and `role=button` on chips that did nothing; and `order` clauses (FEAT-004 §3.1) hit `unknown_clause` rather than a named refusal. One refusal message was *corrected rather than removed*: `crossed_above` now names what is still missing (the validator accepts only `feature`/`bar`, and `sim.rs:1218` still uses the stateless `evaluate_signals`) instead of the missing bytecode op the runtime has since gained. | strategy-lang 155, strategy-validator 81 over 12 targets, clippy clean; frontend 679 tests in 68 files (was 609/63), `tsc` clean, eslint 0 problems, build green; S-5.6, S-5.7, S-6.8, S-6.17, S-6.22 → `DONE`; Set S 31% → 29%, 90/189; phase 5 20% → 33%, phase 6 33% → 46%; headline 39% → 37% |
| 2026-09-27 | **The last `BLOCKED:UPSTREAM` in Set S is gone, and the five stranded agent tracks all landed.** Four concurrent tracks plus a salvage pass. What was blocked and now is not: a perturbation runtime mode (built in `crates/backtest`, and the phase file's own prescription — a shift inside `features::runtime::backfill_column` — was **refused**, because it would hand every feature implementation the ability to read forward); `GET /api/backtest/experiments/{id}/returns`, which `recorded_returns` had only ever reached as a parquet file written for a Python template; and `by_regime`, which reported `Unmeasured` on every bundle this platform has ever served because its only caller passed `&[]`. S-3.17 was not blocked at all — `detect_drift` was on disk and catalogued behind the marker. **Four real defects closed on the way**: the trade rows the job path threw away (`manager.rs` called `run_simulation` where `run_simulation_detailed` had been harvesting round trips all along, which is why all 11 `backtest_jobs` rows report `has_trades = false`); the `pg_notify` two migrations promised with **nothing anywhere issuing a LISTEN**, so every kill-switch trip waited a full cadence against a COMP-007 exit criterion that says "within one event"; 46 mangled string literals in `crates/harness`, concentrated in the tool descriptions that are the whole of what the model is told a tool is *for*; and an approval card that read "`open_vault` falls under the `open_vault` approval envelope" for every approval ever shown to a human. `npm run lint` reached **zero** from ~140, and five of those findings were user-visible bugs — a chart drawing the wrong instrument's live prices for one render after a symbol switch, an order book showing the previous market's depth under the new market's header, sort-header focus destroyed every 120s refetch | 39% left (strict 62 / generous 16); Set S 32% → 31%; phase 3 42% → 32%, blocked 8 → 0; phase 10 37% → 39% (S-10.14 downgraded); harness 770, backtest 474, mcp-server 286, api 421, monitor 281, collectors 193, research-session 291, frontend 609, all green |
| 2026-09-23 | **Phase 6 audited a second time; two more under-claimed, and the same reconciliation mistake repeated once.** S-6.7 (composer) and S-6.23 (provenance fence) had every acceptance clause met against real production callers and move to `DONE`. Four notes corrected without a marker change: S-6.1's real blocker is `npm run lint` failing with ~140 React-Compiler errors that no prior audit ever ran, not a mid-edit tree; S-6.6 is a one-day-old built-but-unreached pair (backend `pause`/`resume` mounted 09-22, frontend still says they're gone from a 09-21 deletion); S-6.22's two remaining named clauses were both closed by commit `e0f9c3f` with the note never updated; S-6.24's stated blocker (missing memory migrations) is false — `desk_digest.rs` already queries `agent_memory.entry` and `monitor.events` successfully, and this file's own later prose already said so without the per-task block being reconciled, repeating a mistake the first audit had already fixed once for S-6.10/S-6.18. Confirmed unchanged: migrations 0119 and 0140 are both applied and successful on the shared dev Postgres with no gaps against disk (the cross-track hazard flagged 09-22 is fully resolved); phase 9 and phase 10 had declared the identical migration range and neither had used it | phase 6: 36% → 33% left; Set S unchanged at 32% (unit-weighted rounding); `scripts/verify_plans.py`: 3 problems → 0 |
| 2026-09-23 | **This file was two days stale, and phase 4 was audited a second time against acceptance criteria rather than code presence.** §1's tables had not been re-derived since 2026-09-21 despite several closings landing on 09-22 (phase 11's five markers, the security fixes, the salvage of thirteen killed agents); re-running the derivation moved phases 0, 3 and 11 along with 4. Phase 4 itself: four `PARTIAL`s were under-claimed (S-4.3, S-4.5, S-4.7, S-4.9 — every clause held, with CI-running evidence, and the note hadn't caught up) and four were over-claimed, where the `PARTIAL` note asserted something false about the tree rather than merely being outdated — S-4.13 grepped `MultiStepEvidence` and missed the `multi_step_evidence` field that's still there; S-4.15 said two files didn't exist when both do, missing the real gap (`conflict::decide_write` has no caller, so `conflicts_with` is a column nothing will ever populate); S-4.17 said only a migration was missing when the sealed eval set actually runs on every PR today, the opposite of its own acceptance; S-4.8 said only a credential was missing when the query function it claimed was tested has zero callers and zero tests. Separately, the silence alarm (`storage::clickhouse::census`, landed 09-22) got its last wire: `health_jobs::run_once` now calls `census::silence_report` on an hourly tick, split from the tenant pass so a Postgres hiccup can't silence the silence alarm | phase 4: 45% → 29% left; Set S 34% → 32%; headline 41% → 40% (strict 63 / generous 18); `cargo test -p platform` 13/13, `cargo test -p storage --lib` 61/61 |
| 2026-09-22 | **Three commits recovered from a worktree, and 26 worktrees retired without losing a line.** Only one of 30 had commits: gate-evidence sources for gates 3, 4, 10 and 12, stranded since 2026-09-15 — `capacity.rs`, `factors.rs`, `ledger/evidence.rs`, migration `0119`, none of which existed on main. One conflict needed a judgement rather than a side: `gate_worker` passed `factors: None` where the incoming commit **supplied** them, which was the commit's whole point. The other 29 held **no commits at all** — a week of agent output visible only as uncommitted edits, including Set P phase 3's collectors for tasks the plan calls unstarted. Each was committed to the branch it was already on, then removed: the worktree is a checkout, the branch is the record | 18 branches hold the preserved work; 22 GB reclaimed; `ledger`, `backtest`, `api` all compile |
| 2026-09-22 | **A session limit killed thirteen agents mid-sentence and every one of them was salvageable.** Twelve crates compiled untouched; the thirteenth did not, and the break *was* the deliverable — `Yield::cost_usd` had been made unrepresentable as a bare float and the caller edits were never named. `dossier::Produced` carried that float into the stored document, which is how every dossier ever compiled came to claim **$0.00** in the one artefact a human reads to decide whether a strategy is worth money | harness 766, backtest 416, mcp-server-lib 266, research-session 211→276, dataplane 205, collectors 145, execution 130, features 109, strategy-runtime 101, storage 73, observability 39 |
| 2026-09-22 | **Two security defects closed.** `/api/backtests/coverage` answered **200 with the platform's entire data inventory** to an anonymous caller — auth here is per-handler, so a route is public by omission and one handler of 234 was missing its extractor. And both WebSocket routes demanded a token neither verified: `ws/live` used the token string **as the user id**, so any non-empty string opened the live feed. The guard resolves every mounted route to its handler and **rejects the words “harmless”, “no data” and “nothing sensitive”** as public-list reasons | CI was running no integration tests at all — `--lib` skips them — so neither this guard nor the route-existence guard had ever run there. Both wired |
| 2026-09-22 | **Five phase-11 markers read `NOT STARTED` over modules of 443–1,063 lines carrying 11–21 tests each.** Corrected against the tree, not a report. None became `DONE`, and the reason is the finding: `session_driver` calls `schedule`, `harvest`, `stall` and `digest` and nothing else, so `sources::ingest`, `couplings::evaluate`, `charter::admit`, `replenish::on_drain` and all of `execute` are complete and unreached. **`charter::StopWhen` had no reader anywhere in the repository** — and `ScheduleEnd` can never fire, because `Schedule` holds weekly windows and no end date, so a charter listing it believes the session will stop and it will not | 41% left (strict 65 / generous 18); Set S 34% |
| 2026-09-22 | **The leakage masking was three deep, and the innermost layer had never run.** `leakage_audit` named `causal_access` and never called it. Two other missing checks held Gate 2 at *inconclusive*, masking that — so closing them first would have let Gate 2 **pass on a check that never ran**. Underneath both, `harness_cv` sized folds needing ≈2.1·n rows for every n, so the geometry never built and **the random-label test returned its did-not-run flag every night since it was written**. And its earned name was a vector length: all-`NaN` statistics with `permuted_max_abs_t` folded to `0.0` over zero finite draws — full-length output, cleanest possible answer | `causal_access`'s behavioural half **cannot fire at all**: `WindowedFrame` keeps `data` and `decision` private and `back(k)` refuses `k >= lookback` |
| 2026-09-21 | **Every open engineering question answered rather than carried — 26 of them, ADR-D-01…D-26.** Three change behaviour in a safety direction; two spec sentences are struck as having **no referent** (“last clause to fail” over a condition whose operands both evaluate; a per-run tool pin the router has no fourth source for); two are **provably wrong rather than ambiguous** — §7.1's cost-drift median cannot fire at any cost ratio, and the validator whitelist refuses the version its own error text recommends, which left the SLv2 authoring path dead at its last step | `OPEN-QUESTIONS.md` has no `Status: open` entries left outside its own template |
| 2026-09-21 | **Waves seven to ten: the audit that wrote no code, and the tracks that shipped nothing.** A read-only track checked every marker in phase 6 against the running platform and moved it 47% → 36% **in both directions** — five tasks done and marked `PARTIAL`, one marked `DONE` failing its own acceptance by 28 files. A catalogue track shipped **no tools** and found that `/api/evidence/trades` had never returned a trade, because `0026` renamed the table out from under its query and the error was swallowed into a 404. A reconciliation track **refused to wire the sweep** it was asked for: the only broker in the process is the paper engine, so the sweep would compare a value with itself and log “reconciliation OK” for a check that cannot fail | frontend 361 → 416 tests in 45 files; `monitor` 167 → 198; `strategy-lang` 129 → 141; `mcp-server-lib` 197 → 209 |
| 2026-09-21 | **Waves three to six: sixteen agents, one worktree, no collisions.** The planted-corpus judgement suite (an answer key that is generated and never stored — no `expected.json`, no `Deserialize`, redacted `Debug`); the four continuous-research modules wired; the desk's three orphan server fields given readers; `/api/desk/running` given a caller. **The worst finding was a reconciliation halt that erased itself within five seconds**, because `KillSwitch::replace_halted` replaces the scoped view every refresh; the second worst was `reorder` marking every id `user`, which switches the scorer off for the session on one drag. **A rule removed in one place and left in two** joined built-but-unreached as a named defect shape: the route learned the new drag rule and the client did not, so one drag posted one id and got a 400 | frontend 193 → 361 tests in 39 files; `harness` 722 → 747; eight spec contradictions opened, none implemented either way |
| 2026-09-21 | **Two waves of parallel tracks, seven agents, one worktree.** Wave one: the strategy language's parser/compiler/decompiler, the stall detector, the digest, the desk's honest-rendering sweep. Wave two: schedule windows, the harvest, the `={null}` audit. **The most dangerous thing found was a kill-switch badge that rendered green *armed* when its endpoint did not answer** — an unknown safety state shown as the safe one, on the control that stops money moving. Also: a flaky test I had shipped (2 failures in 8 runs), a 61-family registry transcription, and four spec contradictions, none implemented either way | frontend 103 → 193 tests; `research-session` 39 → 108; `strategy-lang` 0 → 75 |
| 2026-09-21 | **Documentation audit: the specs were lying about themselves.** Thirteen specs said `Proposed … not implemented` about code that was serving traffic — AGENT-005…014, COMP-005, COMP-007, DATA-006. All now say what is built and what is not, and **point at the phase file for the count instead of restating one**, because a number written into a spec is a number nobody recomputes. Two phase headlines were stale (phase 6 said 1/24 with 2 done; phase 10 said 5/16 with 6). Set S MASTER's status column was stale for eleven of twelve rows. ROADMAP §2 said Set S was untouched | `the_plan_counts_what_the_plans_say` now fails the build on any of it |
| 2026-09-21 | **Set S phase 11 opened: S-11.1, S-11.3, S-11.14 → `PARTIAL`.** `crates/research-session` (the charter with the exploration floor as a sealed type, the agenda, the fold), migration `0138`, the session driver registered as `JobKind::ContinuousResearch`, the full §10 API, three `lifecycle` read tools, the scored agenda with its forced exploration share and `mlops.decision` rows | 211 suites / 3720 tests green; 7 live-Postgres invariants; ADR-P11-01, ADR-P11-02; Q-S11-01 opened |
| 2026-09-21 | The desk's four forward references to `/api/research/sessions` became served; `UNBUILT` rows removed | `the_desk_calls_routes_that_exist` green |
| 2026-09-19 | Set J complete (59/59) | Set J MASTER |
| 2026-09-17 | MLOps consolidation; platform on 7080 | Set I/J MASTERs |
| 2026-09-15 | ROADMAP written; sets P, S, T cut | ROADMAP §13 |

---

## 7. Re-deriving the numbers

Do not edit §1 by hand. Run:

```bash
cargo test -p invariants --test the_plan_counts_what_the_plans_say -- --nocapture
```

It re-derives every count from the plan files and prints the tables. When it
fails, this file is stale and the failure says by how much.
