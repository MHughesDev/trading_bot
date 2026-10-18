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

## 1. Where we are — **22% left**

Derived 2026-10-05 from the `### <TASK-ID>` headings and their status markers
across sets P, S, T, U, M, N, Q, R-a and R-b (442 tasks). Size-weighted **S = 1, M = 2, L = 4**:
tasks of wildly different sizes averaged flat would flatter us.

| counting rule | % of the work left |
|---|---|
| strict — only `DONE` counts | 37% |
| **`PARTIAL` counts half** | **22%** |
| generous — `PARTIAL` counts done | 7% |

Take the middle number and treat it as **±10**, not as a measurement. `PARTIAL`
is 312 of 1028 units and in this repository it spans "complete, one database test
unwritten" to "the types exist and nothing calls them". The spread between the
strict and generous rows *is* the uncertainty, and it is honest to show it rather
than pick one and present it as precision.

**2026-10-05 (track VERIFY2): the headline moved from 40% to 22% left, and this time the work moved it.**
The plan did not grow (still 442 tasks, 1028 units); the push of 2026-10-04/05 closed or advanced
work in Sets M, N, Q, R-a and R-b, which had been "almost entirely ahead of us". Their `DONE` units went
13 → 132 of 348 (tasks 9 → 64 of 148), and their `PARTIAL` units 43 → 160. The older sets moved too:
Set S 137 → 140 of 189 tasks `DONE` (14% left, unchanged), Set T 6 → 7 of 17 (35% → 32%), Set P and
Set U unchanged (17% and 6%). Read the movement with the spread, not the headline alone: **the strict row went
50% → 37% and the generous row 30% → 7%**, so most of the gain is `PARTIAL` work (312 units, 30% of
the programme), and `PARTIAL` here includes "built, tested, and with no production caller yet". The
by-set rows are below. What is left in M, N, Q, R-a and R-b is now mostly integration (the Studio
frontend is 0 of 9 tasks `DONE` and 22 `PARTIAL` units) and the parts only a person or time can
close.

**2026-10-04 (track VERIFY): the headline moved from 19% to 40% and the work did not regress.**
The plan grew. Sets M, N, Q, R-a and R-b were planned on 2026-10-04 (they were reserved letters
in ROADMAP §5 with spec content and no plan files) and joined the derivation the same day:
**5 sets, 148 tasks, 348 units, 13 of them `DONE`** (9 of 148 tasks) — almost entirely ahead of us, so the
headline carries them at their true weight. Over the same push the four older sets
**improved**: Set S 29% → 14% left, Set P 57% → 17%, Set T 85% → 35%, Set U 13% → 6% (the
tracks of 2026-10-03/04 took Set S from 90 to 137 of its 189 tasks `DONE`). Read the
two together: **the four old sets are 14% left and the whole programme is 40%**, and the difference
is the capability work in M, N, Q, R-a and R-b that nobody had scheduled.

**The headline moved from 35% to 49% on 2026-10-01 because the plan grew, not because work
regressed.** Set U (137 units) was added after an audit found that strategies are evaluated by three
different programs, that indicators are never computed on paper, and that no strategy can call a
model in any mode — work no previous set had planned. See [Set U](plan-sets/set-U/MASTER.md).

### By set

| set | subject | tasks | units | done | partial | % left |
|---|---|---:|---:|---:|---:|---:|
| **S** | The Agent Platform | 189 | 446 | 325 | 119 | **14%** |
| **P** | Data Reach | 30 | 51 | 38 | 9 | **17%** |
| **T** | Forward Evidence | 17 | 34 | 14 | 18 | **32%** |
| **U** | Unified Strategy Engine | 58 | 149 | 137 | 6 | **6%** |
| **M** | Engine truthfulness (backtest) | 29 | 67 | 15 | 36 | **51%** |
| **N** | Model platform v2 (AI Model Studio) | 41 | 99 | 41 | 53 | **32%** |
| **Q** | Evaluation tools and evidence extensions | 20 | 39 | 22 | 8 | **33%** |
| **R-a** | Skill registry and materialiser | 24 | 50 | 25 | 23 | **27%** |
| **R-b** | Knowledge, learned tiers, random arm | 34 | 93 | 29 | 40 | **47%** |

P and T are 8% of the weight, and they are the two that carry a **calendar**, which is why §3
puts them first despite Set S being where the code is. They are no longer "almost entirely ahead of
us" (P is 17% left, T 32%): what is left in them is mostly the part only time or a human can
close (a first paper deployment, the archive files, the credentials).

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
| 0 — Ground rules | 6 | 7 | 4 | 3 | 0 | 0 | 21% |
| 1 — Contracts | 20 | 34 | 30 | 4 | 0 | 0 | 6% |
| 2 — One runtime | 16 | 32 | 12 | 20 | 0 | 0 | 31% |
| 3 — The catalogue | 19 | 50 | 36 | 14 | 0 | 0 | 14% |
| 4 — Memory and evaluation | 17 | 39 | 28 | 9 | 0 | 2 | 17% |
| 5 — The language | 15 | 36 | 31 | 5 | 0 | 0 | 7% |
| 6 — The Research Desk | 24 | 66 | 56 | 10 | 0 | 0 | 8% |
| 7 — The monitor | 13 | 29 | 29 | 0 | 0 | 0 | 0% |
| 8 — Integration and honesty | 9 | 24 | 18 | 6 | 0 | 0 | 12% |
| 9 — Context as retrieval | 20 | 56 | 48 | 8 | 0 | 0 | 7% |
| 10 — Modes and the research lifecycle | 16 | 41 | 11 | 30 | 0 | 0 | 37% |
| 11 — Continuous research: Research with the toggle on | 14 | 32 | 22 | 10 | 0 | 0 | 16% |

Derived 2026-10-05 by track VERIFY2 (the 2026-10-04 table read the same except phase 4: 24 done, 13 partial, 22% left, and phase 6: 54 done, 12 partial, 9% left). The narrative below is **history** (2026-09-21 to 2026-09-27) and its percentages are those days', not today's.

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

### Every other set by phase

| set | phase | tasks | units | done | partial | not started | blocked | % left |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| P | 0 — Measure, and start the irreversible clock | 6 | 10 | 9 | 0 | 0 | 1 | 10% |
| P | 1 — Deep backfill | 7 | 13 | 9 | 4 | 0 | 0 | 15% |
| P | 2 — Quality and eligibility | 6 | 10 | 10 | 0 | 0 | 0 | 0% |
| P | 3 — Alternative data | 6 | 9 | 1 | 5 | 0 | 3 | 61% |
| P | 4 — Derived access and the source fence | 5 | 9 | 9 | 0 | 0 | 0 | 0% |
| T | 0 — Reconciliation and the first observation | 4 | 8 | 4 | 2 | 0 | 2 | 38% |
| T | 1 — The deployment path | 5 | 10 | 2 | 8 | 0 | 0 | 40% |
| T | 2 — The paper fleet | 4 | 8 | 6 | 2 | 0 | 0 | 12% |
| T | 3 — Ramp mechanics | 4 | 8 | 2 | 6 | 0 | 0 | 38% |
| U | 0 — The contracts | 5 | 10 | 10 | 0 | 0 | 0 | 0% |
| U | 1 — The engine core | 9 | 25 | 25 | 0 | 0 | 0 | 0% |
| U | 2 — Venues and the risk gate | 6 | 15 | 15 | 0 | 0 | 0 | 0% |
| U | 3 — The backtest environment | 5 | 12 | 12 | 0 | 0 | 0 | 0% |
| U | 4 — Paper and live | 7 | 16 | 16 | 0 | 0 | 0 | 0% |
| U | 5 — Models | 8 | 22 | 18 | 4 | 0 | 0 | 9% |
| U | 6 — Selection strategies | 6 | 18 | 18 | 0 | 0 | 0 | 0% |
| U | 7 — Authoring — identical for people and models | 7 | 22 | 20 | 0 | 2 | 0 | 9% |
| U | 8 — Parity, proven; old paths retired | 5 | 9 | 3 | 2 | 0 | 4 | 56% |
| M | 0 — Reconciliation: what Set U left, what is still owed | 4 | 5 | 2 | 3 | 0 | 0 | 30% |
| M | 1 — Layer 1: research strategies as position series | 8 | 16 | 2 | 14 | 0 | 0 | 44% |
| M | 2 — Costs, RunConfig and outputs | 8 | 19 | 6 | 9 | 4 | 0 | 45% |
| M | 3 — Throughput tiers and tick replay | 5 | 17 | 5 | 4 | 0 | 8 | 59% |
| M | 4 — Research-to-deploy reconciliation and the reference venue | 4 | 10 | 0 | 6 | 0 | 4 | 70% |
| N | 0 — Reconciliation: two DatasetSpecs, a stale Set H, a GPU question | 3 | 4 | 4 | 0 | 0 | 0 | 0% |
| N | 1 — The dataset declaration and `dataset_build` | 7 | 17 | 11 | 6 | 0 | 0 | 18% |
| N | 2 — Model families and bring-your-own | 10 | 30 | 16 | 14 | 0 | 0 | 23% |
| N | 3 — Scoring additions and the hard usage rules | 6 | 13 | 0 | 11 | 2 | 0 | 58% |
| N | 4 — Calibration, serving and operations | 6 | 13 | 10 | 0 | 3 | 0 | 23% |
| N | 5 — The model studio frontend (Set H 5, re-cut) | 9 | 22 | 0 | 22 | 0 | 0 | 50% |
| Q | 1 — The §8 evaluation tools | 12 | 25 | 17 | 6 | 0 | 2 | 20% |
| Q | 2 — Evidence-standard extensions | 8 | 14 | 5 | 2 | 1 | 6 | 57% |
| R-a | 1 — The registry and admission | 5 | 10 | 10 | 0 | 0 | 0 | 0% |
| R-a | 2 — The materialiser | 6 | 10 | 4 | 4 | 0 | 2 | 40% |
| R-a | 3 — Propose, verify and admit | 8 | 19 | 6 | 13 | 0 | 0 | 34% |
| R-a | 4 — Search, the Skills page and SKILL.md procedures | 5 | 11 | 5 | 6 | 0 | 0 | 27% |
| R-b | 1 — Knowledge and memory (KM-02 to KM-10) | 9 | 18 | 10 | 8 | 0 | 0 | 22% |
| R-b | 2 — Skill promotion, curation and evaluation | 6 | 15 | 9 | 6 | 0 | 0 | 20% |
| R-b | 3 — The shadow random-search arm (EV-09) | 3 | 7 | 3 | 4 | 0 | 0 | 29% |
| R-b | 4 — The regime engine and its learned tier | 6 | 15 | 1 | 14 | 0 | 0 | 47% |
| R-b | 5 — Learned model tiers, FEAT-003 phases 2 to 4 and the visual pipeline builder | 10 | 38 | 6 | 8 | 0 | 24 | 74% |

### The older sets, and what is genuinely left in them

Sets A–L use a different format (one `Completion:` line per MASTER, no per-task
markers) so they are not in the headline percentage. Most are closed; **two are not**, and saying
"A–L are done" would be wrong:

| set | state | what remains |
|---|---|---|
| A, B | superseded / complete | nothing |
| C | **CLOSED**, 54/68 shipped | 2 items worth doing (`ws/live.rs` per-subscriber clone), 1 nobody filed (`hot_path.rs` builds a `json!` per trade tick inside the allocation-free pipeline) |
| D | 31/33 (94%) | 2 deferred, recorded |
| **E** | **7/30 (23%)**, 6 partial, 16 open | production readiness. **10 items block unattended paper trading and 4 block a paper fill at all** — those four are Stage 0 work, not Stage 3 |
| F, G, I, J, K | closed / complete / superseded | F has 2 deliberately reversed; G has 4 hygiene items |
| **H** | **47/64 (73%)** | AI Model Studio phases 4–5, folded into Set N (now planned and counted above) |
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
| H-1 | **Deploy one hand-authored strategy to paper** — *no longer needs a credential: paper runs in the app (ADR-0048, 2026-10-02); what blocks it is T-0.4's recorded engineering* | The evidence clock. DONE-3 is the only condition with a calendar in it, and this is the day it starts. **Its engineering prerequisite is discharged as of 2026-09-27**: reconciliation is now installed on the fill stream at startup, so a deployment will actually produce `forward_observation` rows. Until 2026-09-27 this gate would have been spent and produced nothing | T-0.4 |
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
| 2026-10-05 | **Final verification pass over the whole push (track VERIFY2): the workspace builds, lints and tests green except what belongs to the Set U session or to a service this machine lacks, and the plan was re-counted: 40% → 22% left.** `cargo check --workspace --all-targets` was green; clippy `-D warnings` was not (about 30 sites in `backtest`, `harness`, `model-registry`, `research`, `ledger`, `jobs`, `mcp-server-lib` and `api`, all fixed; one was real, a dead `si/pi/fi -= 1` in `harness::memory::curator::shrink_to_cap` whose values were never read). Defects found by running the suites against a migrated scratch database, not by reading: **(1) `session_driver_veto_pg.rs` was a test module living in `crates/api/src/`**, so the boot-sequence invariant (`every_production_binary_that_can_place_an_order_is_accounted_for`) scanned it as a production binary that builds an `ExecutionEngine` without installing forward recording; it moved to `crates/api/tests/inline/`; **(2) its scratch-database helper applied every migration without `_sqlx_migrations`, so 0044 failed and was silently rolled back**, no table made after it carried a grant to `app_role`, and the veto route answered 500 `permission denied for table research_sessions` (fixed with the stand-in table `pg_self_monitor.rs` already uses); **(3) `deployment_gate_switch` and nineteen append-only tables were writable by `app_role` beyond what their migrations granted**, because 0044's default privileges hand `SELECT, INSERT, UPDATE, DELETE` to every new `public` table and a narrower `GRANT` cannot take one away (the lesson of 0167, repeated for 0157, 0170, 0177, 0180, 0181, 0182, 0220, 0222 and 0238): **migration 0246** revokes the surplus, and `deployment_path`'s own probe (an `INSERT` into `deployment_gate_switch` as `app_role` must fail) is what it satisfies; **(4) `pg_model_ops` inserted a `strategy_definitions` row with no `head_revision`**, which 0140 refuses (it now inserts the revision first); **(5) the promoted-set invariant carried a stale `ALLOWED` entry**: `promote_model_version` has filed a proposal since N-4.3 and no longer writes the promoted set, so the entry came off. Line endings: 24 files whose working copy had flipped to CRLF against an LF HEAD were normalised back. Ten stale phase and MASTER headlines were re-counted (four MASTERs, six phase files): Set S MASTER read 137 / 189, holds 140; Set M 2 / 29, holds 7; Set Q 5 / 20, holds 11; Set T 6 / 17, holds 7. **The headline moved because the work moved: Sets M, N, Q, R-a and R-b went from 13 to 132 `DONE` units of 348 and from 43 to 160 `PARTIAL`; the strict row went 50% → 37% and the generous 30% → 7%, so the spread is wider, not narrower.** | 163 migration files (0246 new) apply in order to an empty database (pgvector columns stood in as `float8[]` on 5433), numbers unique, all LF, every new table has `app_role` access and no unintended `DELETE`. Tests per crate, default run: domain 226, execution 218, risk 117, reconciliation 102, monitor 267 (`--test-threads=1`, fresh DB), ledger 231, jobs 129, storage 107, dataplane 323, collectors 246, features 145, backtest 671, research 151, research-session 308, harness 833, model-registry 165, skills 115, strategy-validator 89, strategy-lang 223, strategy-engine 91 (Set U), mcp-server-lib 425, api 755, platform 64, xtask 25, invariants 139; `--ignored` against the scratch database: execution 4, ledger 45, harness 6, skills 22, collectors 1, backtest 6 (+ 5 on a database without `strategy_revision`), model-registry 21 of 25 (the other 4 need the inference sidecar), api 159 of 161 (the other two need pgvector and an illustrative doctest). Frontend: `tsc` 0 errors, vitest 1059 in 114 files, lint 0, build ok; model-trainer 273, model-inference 26 (1 skipped), skill-runner 51, tbot 6, evals 49. Not green and not fixable here: `skeletons_on_btc_usd` (Set U's, `mean_reversion` never fires on 2024 BTC hourly bars), four `storage` legacy suites (a `ledger_events` table no migration creates; `registry_seed` expects 8 asset classes), the live sidecar tests (no inference service on 8002) |
| 2026-10-04 | **The whole push was built and tested together for the first time, and re-counted.** Twenty tracks and the Set U session had been landing in one uncommitted tree; nobody had run the workspace as a unit. `cargo check --workspace --all-targets` was green; clippy `-D warnings` was not (about 100 sites in `ledger`, `harness`, `backtest`, `model-registry`, `reconciliation`, `execution`, `skills`, `features`, `research`, `invariants`, `api` and `xtask`, all fixed except four in Set U files). Real defects found by running the suites, not by reading: a **deliberate-panic experiment left in `pg_strategy_revision.rs`** (it turned the chain test into a guaranteed failure); `precapital_battery` runnable but not demanded by the Significance gate (`enforced: false` against a runnable battery, now `true`, and its pinned unit test updated); two storage and api tests that broke on this push's own migrations (`pg_instruments` deleted a row 0155 now refuses; `projects.rs` inserted a counted job without the trial 0045 requires); `pg_agent_effect_log` counting every suite's `test-fixture-` users instead of its own; and the AT-01 invariant naming `canonical_read.rs`, `qc_reports.rs` and its live test, now allow-listed with a recorded reason each. Sets M, N, Q, R-a and R-b joined `LIVE_SETS` and §1, and seven stale phase and MASTER headlines were re-counted (Set S MASTER read 90 / 189, the tree holds 137; Set T MASTER read 1 / 17, holds 6). **The headline went 19% → 40% left because 148 planned tasks were added, not because anything regressed**: the four older sets went 29/57/85/13% → 14/17/35/6%. | 143 migrations apply in order to an empty database (pgvector columns stood in as `float8[]` on 5433), numbers unique, gaps only the reserved bands, every new table GRANTs `app_role`, all LF. Tests per crate: domain 226, execution 218, risk 117, reconciliation 102, monitor 267 (`--test-threads=1`, fresh DB), ledger 224, jobs 121, storage 106, dataplane 311, collectors 246, features 145, backtest 576, research 90, research-session 308, harness 806, model-registry 112, skills 77, strategy-validator 89, strategy-lang 223, mcp-server-lib 378, api 700 (+107 `--ignored` against a scratch Postgres), platform 63, xtask 25, invariants 129 with 2 failing on Set U's `propose_deployment` schema; frontend vitest 931, lint 0, build ok, `tsc` 1 error in `StrategyGraphPage.test.tsx` (Set U); model-trainer 117, skill-runner 34 |
| 2026-10-04 | **Set U U-5.2 done: an embedding model answers a strategy with its vector.** An `embedding` version is an adapter served by the inference service's `/v2/embed` through the local Ollama embedding model (no key, no cost, nothing downloaded); paper deployments reach it through the gateway, the vector typed against the contract, and a strategy can compare two embedded texts with `cosine`. | `test_embed.py` 5/5 incl. the local `nomic-embed-text` (similar texts closer); gateway `an_embeddings_vector_is_consumed_by_a_strategy` (cosine > 0.8 enters, dissimilar holds, a client that cannot embed is unavailable by name); `live_sidecar_v2` embedding test through the running service: 768-d vectors, rates~rates 0.686 vs rates~bread 0.290; model-registry provider 15/15; Set U 9% left |
| 2026-10-04 | **Set U U-3.4 and U-5.5 done: a v3 backtest runs over HTTP, and the prediction series runs against ClickHouse — without starting the platform.** The platform's own `AppState` and router, built in process as `main.rs` builds them with nothing spawned, take `POST /api/backtests` with an inline graph; the job loads stored bars from a scratch ClickHouse copy and runs on the strategy engine, and the tile reads back `completed` over HTTP. The series job fills a scratch ClickHouse series from a registry version with fold models, each row tagged by the model that answered it, and a refill asks the inference service nothing. | `api/tests/v3_backtest_over_http.rs` (scratch PostgreSQL with every migration, BTC-USD hourly bars 2026-09-01..11: 239 decisions, 6 orders, 3 trades; a bad graph and a wrong timeframe 422, no session 401, another user 404); `platform` `series_runner` against ClickHouse (166 answers, three models each in its window, 3 refused, refill 0 calls); Set U 11% left |
| 2026-10-04 | **Set U U-5.8 (backtest side): every walk-forward fold's own model answers its window.** A version can keep each fold's model as a `model_fold` artifact with its own training end (0189); the point-in-time chooser answers each backtest decision with the latest model — a fold's or the version's own — trained before the decision minus the embargo, and the manifest names the fold. The trainer that fits every fold and the platform's recording of it are built in another session's files and land with its commit (U-5.8 PARTIAL until then). **Found on the way:** no trained model could ever answer a backtest or a paper deployment — the registry read joined on `model_versions.artifact_id`, which training has never set; it now picks the version's `model` artifact by (model, version), and the series rows record which training end answered. Also: a v1 decision model's class map translates to v3 per-class conditions (U-7.3), and v3 strategies arm from automations and sweep in research. | `v3_models` 9/9 (a fold version over 120 bars answers every decision after fold 0 opens, each by the latest model trained before it — dropping the embargo from the chooser fails it; its prediction series covers the folds' union once, each row tagged by its fold); chooser unit test; `pg_fold_artifacts` on scratch PostgreSQL 17 with 0017/0018/0146/0189; `model-registry` provider 14/14; trainer pytest 117 passed incl. `test_fold_models` 5/5; registry read checked 16/16 versions on the platform database (read-only); staged tree checked in isolation; Set U 13% left |
| 2026-10-04 | **Set U U-6.3: a screener someone watches runs on its own schedule.** `selection_watch` (0187) pins a selection's revision and `triggers.schedule`; the platform runs every watch due in a minute through the one `run_selection` and journals it as a scheduled run, once per fire; `POST/GET/DELETE /api/selections/watch(es)`. The selection path ran against ClickHouse for the first time: members load as the store stood at the instant, an unknown symbol is excluded for having no data, the top close is selected. Phase 6 complete. | migration 0187 on scratch PG17 (one running watch per person and revision; stopped once, never edited or deleted; app_role without DELETE); `screener_jobs` 3/3 (due exactly when the schedule fires; spawned at boot); `selection_clickhouse` against the server (2026-10-02 20:00 UTC: BTC-USD and ETH-USD 3 bars each, selected BTC-USD); route invariants 17 + 10; platform 61; Set U 14% → 13% |
| 2026-10-04 | **Set U U-6.6: a trading strategy deployed on a selection reads what the selection said about its instrument.** `selection_attr(selection, attr)` compiles to a plan node that reads a `SelectionContext` on the member's portfolio view — set after every rebalance by the portfolio backtest and by the live allocator through one conversion (`allocation::member_attributes`), so a member reads the same number in both. Unbound, bound to another selection, or given no such attribute: missing, with the reason. **Found by its test:** an attribute read back through `Decimal::to_f64` lost its last digit; it now reads back exactly. Also, beside Set U: the backtest that runs beside a paper deployment (T-0.4's expected series, proven equal to `runner::run`), forward recording wired into deployments and the paper runner, the forward fills and marks a resume needs (0186), a phantom-short fix in that wiring, and `RunConfig::fill_delay_bars` for timing-jitter studies. | backtest `portfolio.rs` 8/8 (every member decision equals its latest selection's attribute exactly; another selection reads missing); platform 59 (paper member reads it exactly; unbound reads missing; forward and replay tests); engine selection 9/9; backtest `shadow` 2/2, `fill_delay` 2/2; storage `pg_forward_fills` on scratch PG17; Set U 15% → 14%; headline 20% → 19% |
| 2026-10-04 | **Set U Phase 8: paper and the backtest are now measured to decide identically.** A paper deployment journals what a replay needs — a genesis on the first record after each start (the warm-up bars, and the checkpoint it restored) and every event it consumed without deciding — and builds its engine through the same call a replay makes, so a segment replays from its genesis through a plan compiled as a backtest compiles it, model answers fed back as received. A nightly job replays the previous day's paper journals and keeps every verdict append-only (0149); a decision that comes out differently is a P1 naming the record and field. **The replay's first run found a real difference**: serde_json without `float_roundtrip` reads some decimals back one ULP off, so a journal read back from JSON drifted in an EMA's last digit at decision 18 — now enabled workspace-wide, with a guard. **A migration tool** translates every stored v1 strategy, re-runs its latest backtest on the engine and publishes old against new; its dry run against the platform database (9 of 14 translate, 2 do not compile, 3 refused by name) **found that `instruments::fetch_by_id` had selected columns that do not exist since June** — every v3 backtest, preview and series run failed on a real database — and that **v1 orders on a signal's rising edge** while the translation ordered on every bar the condition held (10 079 orders for 333); both fixed, and every remaining difference is attributed (`bars_differ`: the store now holds 10 079 bars of a week the old run saw 6 022 of). **U-8.5 PARTIAL** (`--apply` not run: v1-only consumers would break); **U-8.3, U-8.4 BLOCKED** on FEAT-007 §12's own condition and on five v1 producers still live. | `live_runner` replay 3/3 (a 120-decision journal, a restart, model answers) and a perturbed engine naming `values.s`; `parity_jobs` 7/7 incl. the boot source scan and a seeded P1; `replay` 3/3 incl. 20 000 float round trips; storage parity and `pg_instruments` on scratch PostgreSQL 17; translator 6/6 incl. rising edge and 1/0 conditions; migrate bin 6/6; platform 48, backtest 451, strategy-lang 156, api bridge 10/10 (the `strategy` namespace back under its cap); Set U 18% → 15%; headline 26% → 20% (includes other in-flight plan updates) |
| 2026-10-03 | **Set U Phase 7: people and models write strategies the same way, through the same doors.** SLv3 is a text form of the strategy graph — not a second program: the parser lowers straight to the graph's JSON and lets `domain` read it, and printing is a fixed point (`parse(print(g)) == g`, same revision). **One validator**: text, raw JSON or a graph built step by step give the same findings for the same mistake, each with a fix and the guide section that teaches it; every one of `strategy-validator`'s 24 codes has an equivalent or a stated reason it cannot be written in v3. **Found and added:** a stop or target placed zero or a negative distance away was caught by nothing, and a parameter default outside its own range was never checked. A builder (`new`, `add_node`, `connect`, …) produces the same graph as the equivalent text. A v1 → v3 translator carries conditions, signals, exits, model nodes, parameters and limits, lists every behaviour change, and refuses by name rather than approximating (a parameter inside a feature name, a units position limit, per-second limits, `risk_unit` sizing, other-instrument inputs, and the decision, LLM, ranking and pipeline nodes). One API (`/api/v3/authoring/*`, `/api/v3/strategies`, `/api/v3/catalogue`) and seven MCP tools that only call it; the catalogue and the agent's guide are generated from the registries; a graph editor at `/strategy/graph` whose ports refuse a wire of the wrong type. **U-7.3 PARTIAL** (not run over the platform's stored definitions; four AI and pipeline node kinds refused); **U-7.7 added**: a deployment proposal a human approves. | `strategy-lang` 150 lib + v3 suites (round trips incl. the spec's own ```` ```slv3 ```` block, validator equivalence, builder parity, translator corpus, guide = registries); `backtest/tests/translated_v1.rs` (a translated EMA cross backtests on the engine); strategy-engine, domain 230, backtest 511, strategy-validator, mcp lib 261, api route invariants green; vitest 11/11, typecheck and lint clean; failures seen in `api --lib`, `invariants` and `mcp templates_run` are in another session's uncommitted files or need a Python template environment; Set U 29% → 18%; headline 34% → 26% (includes other in-flight plan updates) |
| 2026-10-02 | **Set U Phase 6: selection strategies — what should we buy — run, deploy and backtest.** The engine's node evaluation became one per-instrument routine that the trading engine and a new `SelectionEngine` both run (every existing trading trace byte-identical); a selection evaluates each universe member on its own bars, computes the `cs_*` operators across them, and accounts for every member — selected with its rank, score and attributes, or excluded with the reason, never ranked as zero. Universes resolve point-in-time on both clocks through one type and one loader (`Membership`, `env::universe::load`); a list or asset class says it is today's membership applied to the past. **A selection is a screener**: `POST /api/selections/run` (now or as of a past instant, reading bars as the world stood then), journaled append-only (0147), and the agent's `run_selection` tool calls the same route. **A deployment can trade whatever a selection picks**: an allocator re-runs the selection and applies the one `allocation::rebalance` — the function the portfolio backtest applies — starting members as child deployments that size against their share of the capital, retiring leavers through their own runner (`close` flattens through the risk gate), and journaling every change (0148). **A portfolio backtest** runs one clock and one account across every member, reachable through the trial ledger (`RunConfig.target`, hash-neutral when absent; Experiments take a `target`). Cron schedules had no evaluator anywhere — backtests refused them; `strategy_engine::schedule::Cron` now drives scheduled selections and rebalances. **Found and fixed:** 0104's universe tables were never granted to the platform's role (every owner-run test passed; granted in 0147, with a `SET ROLE app_role` test that fails without it); Phase 5's `/api/models/transforms` shadowed `/api/models/{id}` because a transform's id was its bare name (now `tfm_<name>`); the compiler's `parse` message pointed at a task that had shipped differently. **U-6.3 PARTIAL** (a selection not bound to a deployment does not run on its own schedule; the route not run against ClickHouse); **U-6.6 added**: a trading strategy reading its selection's attributes (`selection_attr`). | `strategy-engine` 53 lib + 8 selection + 22/2/1 suites (cs ranks vs reference incl. a tie, order-independent digest, per-member state, models per member); `backtest/tests/portfolio.rs` 6/6 (a universe mutation fails four), `selection_run.rs` 4/4; allocator 6/6 and two member tests on the real paper engine; PostgreSQL 17 scratch through 0148: `pg_universe` 2/2, `pg_strategy_deployment` 4/4, production member port; live transform test re-run against the inference service; workspace check clean; api 423 + suites, backtest 440, harness 647, mcp 260, platform 32, storage, strategy-runtime, model-registry, jobs, domain green; clippy clean on touched crates; Set U 39% → 29%; headline 36% → 34% |
| 2026-10-02 | **Set U Phase 5: strategies call models, and a backtest cannot see a model trained on its future.** Every model version now carries a typed input/output contract (0146: derived from its kind at boot and on every newly trained version, or declared), and the strategy compiler resolves `name@alias` to a pinned version at compile time; a version with no contract is refused by name. **Point-in-time:** a version answers a backtest decision at `t` only when it finished training (training *or calibration* rows) plus its embargo before `t`; an unknown training end answers nothing; `models_optimistic` is an unsafe flag in the run id, and Gate 0 rejects any run that consumed a future-trained answer, naming the first timestamp. **Serving protocol v2** answers named outputs typed against the contract, and **the v1 zero-fill is gone** — a missing feature abstains by name, `/predict` answers 422 instead of a flat forecast. On paper, a gateway answers each deployment's pinned versions, one call per version per tick. LLM replies are parsed strictly into typed fields and per-call/per-day cost caps abstain before a call is made. Classifiers (probabilities), regressors (raw value — **the trainer had been training every regressor as a classifier**) and owner-registered Python transforms (dry-run against their contract at registration, `POST /api/models/transforms`) all drive strategies. A prediction-series job writes a version's out-of-sample answers to ClickHouse so later backtests ask nothing; it needed a Trainer worker pool, which did not exist. **U-5.2 PARTIAL** (no embedding predictor exists to serve); **U-5.5 PARTIAL** (the ClickHouse path has not run against a server); **U-5.8 added**: training fits one model on the first fold only, so a long backtest holds after it — one model per walk-forward fold is the fix. | `backtest/tests/v3_models.rs` 7/7 (a removed PIT rule fails three); `-p model-registry` 59 + 2 ignored live tests run against the inference service and a scratch PostgreSQL 17 with 0146 (real LightGBM regressor typed; a short row abstains naming `atr_14`; a transform registered, refused when wrong-shaped, compiled against); `-p backtest` 439, `-p platform` 24, `-p jobs` 69, `-p api --lib` 421; inference 10, trainer 70; clippy clean on touched crates; Set U 48% → 39%; headline 38% → 36% |
| 2026-10-02 | **Set U Phase 4: v3 strategies run on paper through the one strategy engine.** `LiveRunner` (the async twin of `cycle`) steps the engine on completed bars of each declared timeframe — folded from the pipeline's minute bars, published to a new `live_bars` hub — and orders through the risk gate and execution engine; models are asked concurrently under their contract timeouts; every decision is journaled (0144, append-only, pruned daily) and checkpointed (0143) — a journal or checkpoint failure halts the deployment; a restart restores and resumes. Deployments are **recorded** (0142, `strategy_deployment`, the writer forward evidence never had), declared in the one registry where stage 3 cannot see them, and resumed at boot; `POST/GET/DELETE /api/deployments`. **U-4.2 PARTIAL** (Set T's readers of the record do not exist yet); **U-4.7 added**: the paper engine has one account per asset class, so deployments share positions and equity. | `apps/platform` 24/24 incl. the live runner's 6 (EMA 7/21 fires from aggregated trades alone; one decision per bar; model timeout holds on time; two models asked at once — a sequential mutation fails it; restart mid-position places the same orders tag for tag; journal failure halts; withdrawal stops within one tick); `checkpoint_restore.rs` (a planner-restore mutation fails it); `pg_strategy_deployment.rs` 3/3 on a scratch PostgreSQL 17 with 0142–0144; strategy-runtime registry tests; `-p api --lib` 421/421; workspace check clean |
| 2026-10-02 | **Set U Phase 3: a v3 strategy backtests through the code that will run it on paper.** `strategy_engine::env` holds the environment ports and `cycle`, the one loop every environment drives; `backtest::runner` runs a compiled graph over every declared input point-in-time on the simulated venue behind the risk gate, as a `RunExecutor` so the trial ledger holds each run before it computes; `RunResult.manifest` records fill policy, model pins, holds and unknowns by reason, gate rejections and a journal digest; a Set J study accepts v3 runs with no code change; preview is the runner's own loop reporting decisions only. Production reach: `AppState` builds `FormatRoutedExecutor` (v3 → new runner, v1 → `market_simulator`), the backtest endpoint and manager take a graph, the preview route takes one. **U-3.4 is `PARTIAL`**: its HTTP criterion was not run, because starting the platform starts live collectors. Model requests are answered `unavailable` until Phase 5. | `backtest/tests/v3_runner.rs` 9/9 (end to end indicator and bracket strategies, multi-timeframe PIT incl. the day-boundary hour, trial ledger, Set J sweep, fill policy, preview = backtest trace for trace, first-fire and held-before reasons, intent-only); `strategy-engine/tests/env_cycle.rs`; mutation on the merge order fails the PIT test; `-p backtest` 437 lib + suites green, `-p api --lib` 421/421, `-p strategy-engine` green; workspace check and clippy clean on touched crates |
| 2026-10-02 | **Set U Phase 2: backtest and paper are one venue with two drivers, and paper trades in the app.** `PaperTradingEngine` reads time and ids only through injected `VenueClock`/`OrderIds`; `SimulatedVenue` drives it from bars on a `SimClock` (next-bar-open fills, stops-first intrabar, `CurrentClose` flagged optimistic); `Venue` is the one port and `PaperVenue`/`SimulatedVenue` both sit behind it. Stop, trailing-stop, reduce-only, post-only and OCO now execute — **every stop-limit in the platform had been a plain limit**, with no field for its trigger, and two broker adapters sent it as one. The production risk gate runs in front of backtests with simulated rate counts and a per-day halt; **the gate's position cap ignored side**, so a strategy at its cap could not sell out — fixed. **Owner decision (ADR-0048): paper trading runs in the app, never on Alpaca's paper account** — `PaperVenue::Platform`, migration 0141, H-7 withdrawn; forward evidence on it is labelled as resting on modelled fills, and fill realism moves to a check on the first live trades. **U-2.6: one fee schedule for both venues** — `FeeSchedule` moved to `domain::fees`, `paper_crypto()` is the one definition the backtest's FeeBook and the paper simulator both read, and resting limits pay maker while crossing orders pay taker in both venues (paper crypto's half-spread moved 5 → 2 bps to match what backtests were already charged). | `cargo test -p execution` all green incl. `tests/sim_venue.rs` 11, `tests/sim_risk.rs` 5, `tests/venue_core_is_environment_neutral.rs` 2; the oracle loses through the venue and wins with costs zeroed; mutations fail the tests they target; `-p risk` 76/76; `-p reconciliation` green; 0141 validated on a scratch PG17 database; `-p backtest --lib` 432/432; workspace check and clippy clean |
| 2026-10-01 | **Set U Phase 1: the one strategy engine exists.** `crates/strategy-engine` — pure, synchronous, deterministic, and depending on nothing an environment owns (a Cargo.lock walk proves it reaches none of execution, risk, storage, model-registry, tokio, sqlx or reqwest). One compiler typechecks every strategy against the operator, indicator and model-contract registries and reports every problem in one pass with did-you-mean fixes. The engine **computes its own indicators from raw bars** through the DATA-006 routine — an EMA-cross fed only bars matches the feature engine bar for bar, and before warm-up its condition is `missing(warming_up)`, not `false` — which is the structural fix for paper trading never computing an indicator. Models are called by request and resume: a classifier feeding a sizing model resolves in two layers and drives an order; an upstream abstain means the downstream model is never called; an incomplete input row is never sent, so nothing is zero-filled. A target sized by a model that timed out holds with that reason. The order planner covers every intent in the terminal's vocabulary: idempotent targets, brackets whose protection is placed on the entry's fill at the fill price, emulated OCO, breakeven stops, and a refusal by name — never a silent downgrade — for an order type a venue lacks. Two mutation checks (temporal state re-stepped on resume; a zero-filled model input) each fail exactly the test written for them. | strategy-engine 38 unit + 22 acceptance tests, clippy clean, no hash-ordered collection anywhere in the crate, byte-identical traces over 150 ticks; Set U 93% → 74%; headline 47% → 43% |
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
