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

## 1. Where we are — **42% left**

Derived 2026-09-21 from the `### <TASK-ID>` headings and their status markers
across sets P, S and T. Size-weighted **S = 1, M = 2, L = 4**: 189 tasks of
wildly different sizes averaged flat would flatter us.

| counting rule | % of the work left |
|---|---|
| strict — only `DONE` counts | 64% |
| **`PARTIAL` counts half** | **42%** |
| generous — `PARTIAL` counts done | 21% |

Take the middle number and treat it as **±10**, not as a measurement. `PARTIAL`
is 227 of 531 units and in this repository it spans "complete, one database test
unwritten" to "the types exist and nothing calls them". The spread between the
strict and generous rows *is* the uncertainty, and it is honest to show it rather
than pick one and present it as precision.

### By set

| set | subject | tasks | units | done | partial | % left |
|---|---|---:|---:|---:|---:|---:|
| **S** | The Agent Platform | 189 | 446 | 84 | 88 | **35%** |
| **P** | Data Reach | 30 | 51 | 5 | 0 | **84%** |
| **T** | Forward Evidence | 17 | 34 | 3 | 0 | **82%** |

P and T are 16% of the weight and almost entirely ahead of us — and they are the
two that carry a **calendar**, which is why §3 puts them first despite Set S
being where the code is.

### Set S by phase

| phase | tasks | units | done | partial | not started | blocked | % left |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1 — Contracts | 20 | 34 | 29 | 5 | 0 | 0 | 7% |
| 9 — Context as retrieval | 20 | 56 | 46 | 10 | 0 | 0 | 9% |
| 8 — Integration and honesty | 9 | 24 | 18 | 6 | 0 | 0 | 12% |
| 7 — The monitor | 13 | 29 | 10 | 18 | 0 | 1 | 34% |
| **6 — The Research Desk** | 24 | **66** | 18 | 48 | 0 | 0 | **36%** |
| 10 — Modes and the lifecycle | 16 | 41 | 11 | 30 | 0 | 0 | 37% |
| 2 — One runtime | 16 | 32 | 9 | 19 | 4 | 0 | 42% |
| 3 — The catalogue | 19 | 50 | 18 | 22 | 0 | **10** | 42% |
| 4 — Memory and evaluation | 17 | 39 | 6 | 31 | 0 | 2 | 45% |
| 5 — The language | 15 | 36 | 12 | 15 | 9 | 0 | 46% |
| 0 — Ground rules | 6 | 7 | 1 | 4 | 1 | 1 | 57% |
| **11 — Continuous research** | 14 | 32 | 0 | 19 | 13 | 0 | **70%** |

The block that matters by *volume* is still **6 — the Research Desk (66
units)**, but it moved 47% → 36% on 2026-09-21 **without anybody writing a
feature**: a read-only audit checked every marker in the file against the tree
and the running platform and found five tasks that were done and marked
`PARTIAL`, and one marked `DONE` that fails its own acceptance by 28 files. The
moves went both ways, which is the point — an audit that only promotes is a
press release.

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
| 1.3 | Memory and evaluation | 4 | ~18 | Feeds DONE-2 and DONE-4 |
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
| H-1 | **Deploy one hand-authored strategy to paper** | The evidence clock. DONE-3 is the only condition with a calendar in it, and this is the day it starts | T-0.4 |
| H-2 | **A frontier provider credential** | No live agent session has ever run. Two Set S acceptance criteria sit behind it, and Set L phases 3–4 are "built, never run live" for the same reason | S-4.14, Set L §4 |
| H-3 | Sort the working tree | Set S phase 0 exit | S-0.1 |
| H-4 | Decide the capital gate | Stage 6 | ROADMAP §8 |

**H-1 and H-2 are the two that matter.** Everything else on this page is work I
can do; those two are work only you can do, and both of them gate conditions that
no amount of building satisfies.

---

## 6. Progress log

Newest first. One line per closing, with the evidence.

| date | what | evidence |
|---|---|---|
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
