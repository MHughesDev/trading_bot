---
Type: Formal
Status: Current
Created: 2026-09-15
Derived From: docs/artifact.md (North Star, SC-8…SC-13, FM-7…FM-11), BS-007 17_BUILD_ORDER §2–§3
  (superseded here), every plan-set MASTER, ml-platform-context-pack HANDOFF/PROGRESS/CURRENT-STATE,
  AGENT-005…014, COMP-007, DATA-005, ADR-0027, ADR-0034…0037, ADR-P2-14
---

> **Current state lives in [PLAN.md](PLAN.md), not here.** This file is the
> *reasoning*: why the stages are ordered by the evidence clock, what "done"
> means, which gates need a human. Its §2 snapshot and §12 "what to do next" are
> point-in-time and go stale between updates — PLAN.md is the one that is kept
> current, carries the counts, and is re-derived from the plan files rather than
> written by hand. Read PLAN.md first; come here for why.

# The Programme Roadmap

**One map, from where the code actually is today to the North Star in
[`docs/artifact.md`](../artifact.md).** It sits *below* the artifact and *above* every
plan-set `MASTER.md`: the artifact says what good looks like, this file says in what order
and why, and each set says how.

> **Authority.** Where this file and a plan-set MASTER disagree about a set's *scope*, this
> file wins and the MASTER is re-derived. Where they disagree about *how a task is done*,
> the MASTER and its specs win. This file never invents work — every item below traces to a
> spec, an ADR or a recorded decision, and where no such source exists yet that is written
> down as an **upstream gap** (§9) rather than planned around.

**It supersedes [`BS-007 17_BUILD_ORDER`](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/17_BUILD_ORDER.MD)
§2 and §3** (the reserved set letters and their dependency graph), which were written before
Set S existed and before the ML platform pack was built. §1, §4, §5 and §6 of that file
still stand.

---

## 1. The one thing this roadmap is organised around

**There are two clocks, and only one of them is engineering.**

The **build clock** is every task in every plan set. It is parallelisable, it is
agent-executable, and more effort makes it go faster.

The **evidence clock** is not. It is:

- **History depth — now measured, 2026-09-15.** The census
  ([P-0.1](plan-sets/set-P/P-0.1-CENSUS-2026-09-15.md)) read every row of `market_bars_v2`:
  **158,259 rows, 4 instruments, 2 timeframes.** The old "22 one-hour bars" anecdote was true
  and badly misleading — it was one instrument, one timeframe, one venue.

  **What matters is the longest *continuous* run, not the row count.** BTC-USD/coinbase/1m
  holds 92,013 bars over a 119-day span, but that is 31 days, a 53-day hole, then a fragment.
  The best continuous stretches anywhere are **88.1 days (BTC-USDT 1h)** and **30.8 days
  (ETH-USD 1m)**.

  **Nothing on the platform satisfies `paper_v1`'s 365 days, let alone `strict_v1`'s 1,826.**
  The best series is 277 days short of paper and 1,738 short of strict. At forward collection
  alone, paper arrives around **June 2027** and strict around **August 2031** — unless Set P
  Phase 1's backfill closes it. That is the evidence clock, as a date rather than a worry.
- **Forward observation.** Gate 15 takes evidence only from observed-knowledge rows. A
  forward test cannot be backfilled, accelerated, or bought. It is a calendar.

  **And the census found the platform currently has none of it.** All 158,259 rows carry
  `available_time = event_time` — minimum and maximum lag both zero — **including the three
  REST backfill sources**. The platform cannot distinguish a bar it watched arrive from one
  handed to it weeks later, so under D-P3 **no existing bar is admissible as forward evidence
  at all**. This is INV-01 violated store-wide: a `knowledge_time` column that is uniformly
  false is worse than an absent one, because embargo computation and every PIT filter read it
  as true. Dispatched as track 20.
- **Series that have no history at all until collection starts.** Funding rates, open
  interest, options chains, order-book depth, news. These are **forward-collect only**.
  There is no vendor to buy 2024 from under a free-data policy. Every day a collector is
  not running is a permanent hole in 2028's research.

That last one is FM-11, and it is the only failure mode in the artifact marked *cannot be
repaired retroactively at any cost*.

**The consequence, and it is the load-bearing decision in this document: work that starts
the evidence clock outranks work that improves research throughput, even though the second
is more interesting and is where the whole design effort has gone.** Set S makes the agent
good. Set P and Set T decide whether there will be anything for a good agent to be right
about. They run *first and concurrently*, not after.

This is not a claim that Set S is less important. It is a claim about *ordering under an
irreversible constraint*: Set S delayed by a quarter costs a quarter of research
throughput; a collector switched on a quarter late costs a permanent quarter-shaped hole in
every study that ever uses that series.

---

## 2. Where we actually are (2026-09-15, **partially superseded 2026-09-21**)

Verified against the repository, not against the plan files' own claims.

> **Six days of drift, corrected in place below.** The rows marked ✎ were wrong
> on 2026-09-21 and have been rewritten; everything unmarked was re-checked and
> still holds. The counts now live in [PLAN.md](PLAN.md), which derives them
> from the plan files instead of restating them by hand — restating them by hand
> is how this section came to claim Set S was untouched six days after it
> started.

### Built and load-bearing

| Area | State | Evidence |
|---|---|---|
| Rust platform | ✎ **33 crates + 17 apps**, serves on 7080, Postgres + ClickHouse + NATS + Redis. Workspace suite green at **211 suites / 3720 tests** | `crates/`, `config/local.toml` |
| Migrations | ✎ **at 0138** (was 0060 on 2026-09-15) | `migrations/` |
| Trial ledger | `crates/ledger` over `mlops`: append-only chained events, sealed `TrialTicket`, typed `N_eff`, return-series CHECK | ADR-P0-12, migrations 0043–0059 |
| INV-16 (no unregistered compute) | **enforced across every dispatch path**, including the legacy UI route under an explicit `legacy_unlogged` marker | OQ-06, OQ-08 |
| Gate funnel | 16 gates, two profiles (`strict_v1`, `paper_v1`); **11 of 16 judge on real numbers** | ADR-P2-14, HANDOFF §4 |
| Feature engine | one windowed implementation, no Python duplicate, serves logged, nightly diff | INV-14, AT-15 |
| Honest-evaluation core | Run/Study/Experiment, Null Library, sealed distributions, holdout vault | Set J, 59/59 |
| Forecasting suite | distributional contract, walk-forward CV, CRPS/PIT, ensembles, pipeline factory | Set I, 77/77 |
| Agent foundation | sandbox, scoped tokens, research cutoff, job service, artifact store, `tbot` read surface, workspace UI, auditor suite in CI at 100% catch | Set L |

### Built but unproven, partial, or stopped

| Gap | Why it matters | Where it is tracked |
|---|---|---|
| **No live agent session has ever run** | needs a provider credential; two Set S acceptance criteria are `BLOCKED:HUMAN` behind it | Set L §4, Set S §7, task S-4.14 |
| ✎ **Campaign child workers: `ResearchRun` now exists** (S-10.11, registered in `apps/platform`); `Study` and `EvalTask` are still unregistered, so a campaign reaching EXPERIMENT or COMPARE still fails with `no_worker` | a campaign fails at its first unserved dispatching phase; nothing consumes `Comparison` | HANDOFF §5.2 |
| **5 of 16 gates have no evidence source** (3, 4, 10, 12, 15) | the funnel cannot render a complete verdict | HANDOFF §5.3 |
| ~~Gate 15 has no input~~ | **built 2026-09-15 (track 5).** Reconciliation is wired, the verdict leads with its observation count and window, and a backfilled row planted inside the window provably does not enter the count. It has never rendered a real verdict because **nothing has produced a fill**. It is blocked on track 16. Gate H-7 was withdrawn on 2026-10-02: under ADR-0048 paper fills come from the in-app paper engine, so no broker credential is needed | Set T Phase 0 |
| **AT-61 unwritten** | no framework can claim `bit_identical` resume, so every resume restarts under the same trial | HANDOFF §5.1 |
| **Phase 3 writers unscheduled** | `asset_embedding` and the outcome tensor have code and tests but nothing writes them | HANDOFF §5.4 |
| **The external MCP front door writes no audit trail** | a named exemption, not an oversight, but a real hole | AT-64 |
| **Data depth unmeasured and believed thin** | gates the entire evidence clock | DATA-005 S1 → **Set P** |
| Four `storage` live suites fail | pre-existing; documented do-not-chase. `lint-no-json-hotpath` was on this row and is **fixed and CI-wired** (ADR-C-LINT-1, on main 2026-10-17) | HANDOFF §3 |
| **Every simulator run ever produced was charged zero commission** | the SDK's instrument constructors leave maker/taker unset and the engine's default fee model reads that as zero, so the simulator computed a **gross** number and labelled it **net**. Found 2026-09-15 by track 15, fixed via `FeeBook::resolve` (ADR-P6-12). Affects every run, not only exit-declaring ones | ADR-P6-12; `mlops.defective_engine_version` + the `backtest_runs_integrity` view separate the populations at read time |
| **Every simulator `Trade` carried a size of zero** | a closed nautilus `Position` reports `quantity` as the *open* size; nothing noticed because no metric divided by it. Found the same day, fixed as `peak_qty` | ADR-P6-13 |
| **Exits fire in replay but not live** | track 15 wired the simulator to `ExitTracker`; the live runtime still does not call it. A declared stop-loss now bounds a backtest and not a deployment — an **SC-3 violation**, and the most dangerous state the platform has been in | dispatched as track 19 |

### Plan sets, honestly

| Set | Subject | State | Disposition |
|---|---|---|---|
| A | Python→Rust rewrite (original) | superseded | closed by B |
| B | Full end-state platform | **Complete** | closed |
| C | Latency optimisation (68 issues) | **54 shipped / 13 open / 1 obsolete** — audited 2026-09-15 with file:line evidence | **CLOSED.** Only 2 open items are worth doing (#14+#15, the `ws/live.rs` per-subscriber clone) plus one nobody filed: `hot_path.rs` builds a `serde_json::json!` per trade tick inside the allocation-free pipeline |
| D | Backtest decoupling/hardening | 31/33 (94%), 2 deferred | closed, deferred items recorded |
| E | Production readiness / tech debt | ✎ **7/30 (23%), 6 partial, 16 open** — its own MASTER, re-read 2026-09-21 | **10 items block unattended paper trading, and 4 of them block a paper fill at all — those are Stage 0, not Stage 3.** Dispatched as track 16 |
| F | MCP platform expansion | **9/11 shipped, 2 deliberately reversed** — the `0/11` was a bookkeeping failure; `INTG-001` has read `Implemented · v2.0 (Set F)` since September | **CLOSED.** The 2 remaining were reversed on purpose: a second `BacktestManager` is the defect ADR-0022 §1 removed, and `create_backtest` was deleted under INV-16 |
| G | Documentation restructuring | **11/13** — executed the day it was written (commit `5febac2`); the task count itself was wrong | **CLOSED**, 4 hygiene items re-cut (including `docs/archive/README.md`'s 24 dead links and the `README.MD`/`README.md` case-fold collision) |
| H | AI Model Studio | 47/64 (73%), Phases 4–5 pending | folded into Set N |
| I | Probabilistic forecasting | **77/77 Complete** | closed |
| J | Honest-evaluation core | **59/59 Complete** | closed |
| K | Make backtest real | superseded | closed by L |
| L | Agent foundation | Phases 0,1,2,5,6 core complete; 3–4 built, never run live | closed; Set S rebuilds on its evidence |
| **S** | **The Agent Platform** | ✎ **72 done · 85 partial · 29 not started · 3 blocked** of 189 (2026-09-21). Phases 1, 8 and 9 are effectively closed; 11 opened | **Stage 1 — in progress** |
| **P** | **Data reach** | ✎ **5/30**, ~84% of its weight remaining | **Stage 0 — barely started, and it gates the evidence clock** |
| **T** | **Forward evidence** | ✎ **3/17**, ~82% remaining; T-0.4 is `BLOCKED:HUMAN` | **Stage 0/2 — blocked at the ops step that starts the clock** |
| M, N, O, Q, R-b | reserved | residual scope re-cut in §5 | Stage 4–5 |
| ML platform pack | Phases 0–5 | 78 done, 19 partial, 5 deferred (all of Phase 6) | remainder folded into Stages 2–3 |

---

## 3. What "done" means for the programme

Not "every set complete" — the sets are a means. The North Star is met when all six hold,
each measured by a mechanism that already exists or is planned below:

| # | Condition | Measured by |
|---|---|---|
| **DONE-1** | A question typed into the desk produces a sealed pre-registration, a platform-computed verdict and a knowledge unit with no human step in between, and the whole run replays byte-identically from its receipt | AGENT-013 §13, `lifecycle.reproduce`, nightly `repro_drift` |
| **DONE-2** | `cost_per_knowledge_unit` is measured per run and its 90-day trend is flat or falling while the knowledge base grows | AGENT-013 §10 yield accounting |
| **DONE-3** | ≥ 1 strategy holds a `paper_v1` pass whose forward fills reconcile with its backtest distribution inside tolerance, **and** ≥ 10 have been rejected with a named failure class that a later run retrieved and did not re-test | Gate 15 + AGENT-007 negative results |
| **DONE-4** | The `judgement` suite — planted edges the agent must find, planted nulls it must reject — scores at or above threshold on the frontier tier, and each local tier reports a measured score or `unmeasured` | AGENT-009 §3 |
| **DONE-5** | No trial exists anywhere without a ledger row and a sealed pre-registration, through every door including the external MCP server and the UI | INV-16 + D-S12 + AT-64 closed |
| **DONE-6** | A continuous session runs a week unattended inside budget and returns a digest a human acts on, having chosen its own agenda and stopped itself when it stalled | AGENT-014 §12 |

DONE-3 is the only one with a calendar in it. That is §1 restated as an acceptance
criterion, and it is why Stage 0 exists.

---

## 4. The stages

Stages are **gates on evidence**, not sprints. Tracks inside a stage run concurrently; a
stage ends when its exit condition is demonstrable, not when its tasks are ticked.

```
         ┌──────────────────────────────────────────────────────────────────┐
Stage 0  │ START THE CLOCKS        Set P Ph.0-1  ·  Set T Ph.0  ·  Set S Ph.0│  weeks
         └──────────────────────────────────────────────────────────────────┘
              │                          │                        │
              ▼                          ▼                        ▼
         ┌─────────────┐        ┌──────────────────┐   ┌────────────────────┐
Stage 1  │ Set P Ph.2-4│        │ Set T Ph.1-2     │   │ SET S  (189 tasks) │
         │ data reach  │        │ forward evidence │   │ five tracks        │
         └─────────────┘        └──────────────────┘   └────────────────────┘
              │                          │                        │
              └──────────────┬───────────┴────────────────────────┘
                             ▼
         ┌──────────────────────────────────────────────────────────────────┐
Stage 2  │ COMPLETE THE EVIDENCE SURFACE   pack remainder · Q-residual · 16/16│
         └──────────────────────────────────────────────────────────────────┘
                             │
                             ▼
         ┌──────────────────────────────────────────────────────────────────┐
Stage 3  │ PAPER AT SCALE    Set T Ph.3   · Set E · COMP-007 live · ramp     │
         └──────────────────────────────────────────────────────────────────┘
                             │
                             ▼
         ┌──────────────────────────────────────────────────────────────────┐
Stage 4  │ CAPABILITY DEPTH    Set M-res · Set N (absorbs H 4-5) · Set O     │
         └──────────────────────────────────────────────────────────────────┘
                             │
                             ▼
         ┌──────────────────────────────────────────────────────────────────┐
Stage 5  │ ACCUMULATED KNOWLEDGE    Set R-b · regime engine · skills lifecycle│
         └──────────────────────────────────────────────────────────────────┘
                             │
                             ▼
         ┌──────────────────────────────────────────────────────────────────┐
Stage 6  │ CAPITAL     human decision gate — Q-4 first, then strict_v1       │
         └──────────────────────────────────────────────────────────────────┘
```

### Stage 0 — Start the clocks

**Goal.** Stop losing information that cannot be recovered. Nothing here is interesting;
all of it is irreversible.

| Track | Work | Set |
|---|---|---|
| Measure | Count what history actually exists, per instrument per timeframe, and publish it | P-0 |
| Backfill | Resumable deep backfill from free endpoints (Coinbase, Kraken, Alpaca) | P-1 |
| **Collect forward** | **Switch on funding, open interest, options chains — the series with no backfill** | P-0.5 |
| Reconcile | Wire `crates/reconciliation` so paper fills produce Gate 15 observations | T-0 |
| Unblock | Resolve the dirty working tree; obtain the model credential; accept ADR-0034…0037 | S-0 |

**Exit.** A coverage report exists and is published; forward-collectors are running and
their first rows are in ClickHouse with honest `knowledge_time`; a paper fill produces a
reconciliation record; Set S Phase 0 is green and five tracks can branch.

**This stage should be measured in weeks and started today.** Its cost is small and its
option value is the entire evidence clock.

### Stage 1 — The agent platform

**Goal.** Set S, in full: one runtime loop, one context budget, one `ToolSpec`, memory,
the eval suite, SLv2, the Research Desk, the monitor, retrieval-based context, the
ten-stage lifecycle with pre-registration, and continuous research.

**Runs concurrently with** Set P Phases 2–4 and Set T Phases 1–2, which are different
crates and different people/agents.

**Exit.** Set S §6 — all eleven specs' acceptance sections pass with evidence, with the
two `BLOCKED:HUMAN` criteria closed by the credential obtained in Stage 0.

### Stage 2 — Complete the evidence surface

**Goal.** Make a verdict *complete*. Today 5 of 16 gates have no evidence source and a
campaign dies at its first dispatch.

| Work | Source |
|---|---|
| Campaign child workers `Study`, `EvalTask`, `ResearchRun` (the last is Set S Phase 10) | HANDOFF §5.2 |
| Gate 3 cost-sweep breakeven, Gate 4 capacity runs, Gate 10 factor returns, Gate 12 per-instrument P&L | HANDOFF §5.3 |
| Gate 15 forward observations | Set T |
| AT-61 bit-identical resume | HANDOFF §5.1 |
| `asset_embedding` + outcome-tensor writers scheduled | HANDOFF §5.4 |
| The 19 partial checklist items closed or re-declared | IMPLEMENTATION-CHECKLIST |
| AT-64: the external MCP front door's audit trail | HANDOFF §4 |
| Set Q residual: EV- extensions beyond Set S Phases 3 and 8 | §5 |

**Exit.** One strategy runs the full funnel end to end and receives a `paper_v1` verdict
computed from 16 real inputs, with no `NotComputable` and no gate judging on a placeholder.

### Stage 3 — Paper at scale

**Goal.** Many candidates on paper, continuously, safely, with the monitor live.

Set T Phase 3 (capital-ramp mechanics), with Phase 2's paper fleet running at scale. Set T has
phases 0–3 only; this line used to cite "Phases 3–4", which do not exist, and was corrected on
2026-10-03. Deployment proposals are Phase 1. Also in this stage: COMP-007 promoted from
"built in Set S Phase 7" to "running in production with drills", and Set E's remaining 25
production-readiness items. Those items become load-bearing the moment something runs
unattended: first on the in-app paper venue (ADR-0048), later against a live broker.

**Exit.** Paper fills reconcile with backtest distributions across a fleet; a kill-switch
drill stops order flow within one cadence; DONE-3's forward-observation count is
accumulating.

### Stage 4 — Capability depth

**Goal.** Remove the ceilings the agent will hit once it is good.

- **Set M residual** — backtest-engine truthfulness beyond the feature engine (Set S Phase
  5 takes the features and the one-engine rule).
- **Set N** — models: `MD-01…MD-13`, FEAT-006, absorbing **Set H Phases 4–5** (inference
  and the studio frontend), plus the analysis fit-process.
- **Set O residual** — SLv2's *visual builder* surface (Set S Phase 5 takes SLv2 as the
  model-facing language, compiler and moves).

**Exit.** The archetype space is bounded by ideas, not by the language or the engine.

### Stage 5 — Accumulated knowledge

**Goal.** The part of the North Star that is *not* "run good experiments" — the part where
knowledge compounds.

**Set R-b** — `KM-02…KM-10`, skills promotion/curator/A-B (`SK-11, 16, 17`), `EV-09`'s
random arm, the regime engine (the one piece Set S explicitly left in R-b), FEAT-003
Phases 2–3 inside the new harness, and the R1 techniques: tree search, move memory,
playbooks, Swiss tournaments, sleep-time compute.

**Exit.** DONE-2 and DONE-4 — yield per unit cost improving, and the `judgement` suite
showing the knowledge base makes decisions *better*, not just faster.

### Stage 6 — Capital

**Not an engineering stage.** It opens only when **Q-4 (capital and liability) is answered
by a human** and a strategy holds a `strict_v1` pass. Ring 3 has no tool, by design and
permanently (D-12, ADR-0025, D-S12). Everything technical for it already exists: the risk
gate chokepoint (SC-2), the kill switch, the capital ramp (10/25/50/100%, Gate 16), the
approval envelope.

---

## 5. Set succession: what the reserved letters still mean

`17_BUILD_ORDER §2` reserved M, N, O, P, Q, R-a, R-b before Set S was cut. Set S is
organised around the agent rather than around subsystems, so it took pieces out of four of
them. This table is the honest remainder, and it replaces that section.

**Updated 2026-10-04 (track F-RESERVED):** M, N, Q, R-a and R-b are no longer reserved
letters. Each now has a `MASTER.md` and phase files with sized tasks, acceptance clauses,
dependencies and migration needs, and each says what Set S and Set U already own so that
nothing is built twice. Task ids are `M-`, `N-`, `Q-`, `RA-` and `RB-` (the id grammar the
count test reads is an uppercase prefix, so `R-a` becomes `RA`). Their counts are held by
`the_plan_counts_what_the_plans_say` as *planned sets*, outside the PLAN.md headline until
they are promoted into its live-set list.

| Letter | Original scope | Taken by Sets S and U | **Residual scope** | Plan | Stage |
|---|---|---|---|---|---|
| **M** Engine truthfulness | `FE-`, `BT-`, `ST-` Layer 1 | the DATA-006 F1/F2 feature engine and the one-engine rule (S Phase 5); the native venue, order vocabulary, fill timing, Layer 2 driver and model leakage check (U) | Layer 1 position series and the Gate-0 truncation test, dated cost models, `RunConfig` honouring, outputs v2, batch and tier-0, tick replay, research-to-deploy reconciliation | [set-M](plan-sets/set-M/MASTER.md) | 4 |
| **N** Models | `MD-01…MD-13`, TB §3.7 | per-fold models, the prediction series, typed model contracts (U) | the dataset **declaration**, the family registry, real sequence windows, scoring additions with the hard usage rules, calibration and serving, **plus Set H Phases 4–5** | [set-N](plan-sets/set-N/MASTER.md) | 4 |
| **O** Strategy Language v2 | `ST-05…ST-12`, `UI-09` | SLv2 as the model-facing language, compiler, moves (S Phase 5); SLv3 and the unified engine (U) | the **visual builder's** v2 surface, `UI-09` — and the SLv2/SLv3 reconciliation (OPEN-TASKS C5) is still undecided | not yet planned | 4 |
| **P** Data reach | `DA-10…DA-12`, S1–S7 | nothing | **whole — written 2026-09-15, [`set-P/MASTER.md`](plan-sets/set-P/MASTER.md)** | [set-P](plan-sets/set-P/MASTER.md) | **0–1** |
| **Q** Research standard | `EV-01`, `EV-03…EV-08` | the honesty arithmetic and the leakage battery (S Phases 3, 8) | the §8 evaluation tools that do not exist (the table has 8 High and 4 Medium, not 11), and the G3 rules that consume them | [set-Q](plan-sets/set-Q/MASTER.md) | 2 |
| **R-a** Skills lifecycle | `SK-02…SK-08, 10, 12…15`, `UI-06` | playbook admission, episodic + semantic memory (S Phase 4); core skills and the v0 materialiser (L, LEGACY) | the registry, admission, the scoped materialiser, propose/verify, search, `UI-06` | [set-R-a](plan-sets/set-R-a/MASTER.md) | 5 |
| **R-b** Knowledge and autonomy | `KM-02…KM-10`, `SK-11/16/17`, `EV-09` | campaigns as the `optimise` workflow; autonomy as continuous research (S Phases 10–11) | knowledge (move memory, curator, analogs, tainting), skill promotion and evaluation, the **random arm**, the **regime engine**, learned tiers behind ledger-size triggers, FEAT-003 phases 2–4 | [set-R-b](plan-sets/set-R-b/MASTER.md) | 5 |
| **T** Forward evidence | was "Track (own set)" | nothing | **whole — written 2026-09-15, [`set-T/MASTER.md`](plan-sets/set-T/MASTER.md)** | [set-T](plan-sets/set-T/MASTER.md) | **0–3** |

Letters already used and closed: A, B, C, D, E, F, G, H, I, J, K, L, S. Letters now planned and open:
M, N, P, Q, R-a, R-b, T, U. The next free letter after U is **V**; Set O is the one reserved
letter still without a plan.

---

## 6. Critical path

The longest chain to DONE-3, which is the only condition with a calendar in it:

```
P-0 measure coverage
  └─▶ P-0.5 forward collectors ON ──────────────────────────────┐ (permanent from here)
  └─▶ P-1 deep backfill ─▶ P-2 quality grades ─▶ dataset depth  │
                                                    │           │
T-0 reconciliation wired ─▶ T-1 paper deployment ───┴──▶ Gate 15 observations accumulate
                                                                │
S-0…S-11 (the agent) ──▶ a candidate worth deploying ───────────┘
                                                                ▼
                                            DONE-3: a paper_v1 pass that reconciles
```

**Two observations that decide the schedule.**

1. **The agent is not on the critical path to first evidence — but something else is.**
   A hand-authored strategy deployed to paper starts the Gate 15 clock just as well as an
   agent-authored one. That part stands. **The "costs a day" part was wrong**, and the
   2026-09-15 audit is what corrected it: *there is currently no way to get a fill out of
   this platform at all.* Hot-path stage 3 is `Option::None` (a started strategy returns 200
   and produces zero intents, silently, forever); armed automations are dead code with no
   caller; `PercentOfBalance` sizing returns `None`; exit rules are dropped on save; and
   repo-wide there is no `INSERT INTO orders` or `INSERT INTO fills`, so nothing would be
   recorded even if it filled. **Set E's execution-path items are therefore Stage 0 work,
   not Stage 3 work** — they sit directly on the critical path to DONE-3, ahead of the agent
   and ahead of everything else on the board. Dispatched as track 16.

   Worth stating plainly because it is the kind of error this roadmap exists to catch: the
   shortcut that took the agent off the critical path assumed a working execution path, and
   nobody had checked. An audit found it in an afternoon. **The assumption, not the agent,
   was the risk.**
2. **`strict_v1`'s five-year floor may be unreachable for years under free-only data.**
   That is exactly what `paper_v1` (ADR-P2-14) exists for, and why Stage 6 is gated on a
   human decision rather than scheduled. Do not lower `strict_v1`; it is immutable by
   design and lowering it would void the platform's whole claim.

---

## 7. Concurrency and collision control

Five Set S tracks plus Set P plus Set T is seven concurrent branches. Collisions are
managed the way Set S already manages its own:

**Migration ranges — extended here.** Latest on disk is `0060_web_and_skills.sql`.

| Track | Range | Source |
|---|---|---|
| Set S Phases 2–11 | **0061–0096** | Set S §5 (nine sub-ranges) |
| **Set P data reach** | **0097–0104** | this file |
| **Set T forward evidence** | **0105–0112** | this file |
| Stage 2 pack remainder | 0113–0120 | reserved |
| Stage 4 (M-res, N, O) | 0121–**0139** | reserved; **exhausted** — `0140` was taken out of band, see below |
| Stage 5 (R-a, R-b) | 0141–0160 | reserved |

**`0140_strategy_definition_head_required.sql` is taken** (2026-09-21), and it is
not Stage 4's. It deletes 18 test-residue rows from `strategy_definitions` and
makes `head_revision` `NOT NULL`. It took the next free number rather than one
past the reservations because sqlx applies any unrecorded file *in file order*
and does not object to a version below the highest already applied — so a file
parked at `0161` would not have been refused, it would have been applied before
`0141–0160` were written and after them everywhere else. The live database and a
freshly migrated one would then hold the same set of migrations applied in two
different orders, which is precisely what `db_invariants`' *"every migration
applies to an empty database"* cannot catch. (Later work has since used numbers
throughout `0141`–`0250`, so the stage reservations above are historical: a new
migration takes the next number above the highest one present, for the reason just
given.)

**Shared files** beyond Set S's list (`Cargo.toml`, `Cargo.lock`, `taxonomy.rs`,
`routes/mod.rs`, `App.tsx`, `TopBar.tsx`), for Sets P and T:
`crates/collectors/src/lib.rs`, `crates/dataplane/src/source.rs`,
`clickhouse/*.sql`, `crates/reconciliation/src/lib.rs`, `crates/execution/src/lib.rs`.

**One branch per phase**, named `claude/set-<x>-phase-<n>-<topic>`; CI runs on `claude/**`.

---

## 8. Decision gates that need a human

Everything else is delegated. These are not.

| Gate | Question | Blocks | Status |
|---|---|---|---|
| ~~H-1~~ | The dirty working tree | — | **CLOSED 2026-09-15, commit `37faade`.** 161 files, +27,518. The two `Cargo.toml` workspace-member lines, the 13 untracked catalogue tools, `ExitTracker`, `FeeBook`, the falsification ledger, `webfetch`, `skills`, migrations 0058–0060, `.gitattributes` — and the plan corpus itself (`ROADMAP`, `EXECUTION-BOARD`, plan-sets P/S/T, AGENT-005…014, ADR-0034–0037), which was also untracked. **Every worktree can now rebase onto a baseline that contains what its task depends on**, and the mirrored "user's work" commits on ~25 branches should be dropped rather than deduplicated at integration |
| **H-2** | A model provider credential in the encrypted store | the **only two** acceptance criteria in all of Set S an agent cannot close (AGENT-005 §10.2's cache-hit measurement, AGENT-009's `decode` suite); every live agent session | **open — track 22 is building everything either side of it** |
| **H-3** | Monthly research budget ceiling (the platform default is $25/day per continuous session) | continuous research at scale | default set, revisit at Stage 2 |
| ~~H-7~~ | ~~**Alpaca paper credentials.**~~ **Withdrawn 2026-10-02 (ADR-0048): the owner decided paper trading runs in the app, so no paper credential is needed.** T-0.4 is now blocked only on its recorded engineering. Original text: Track 5 built the whole forward-evidence pipeline and cannot start it: T-0.4 is blocked on a paper fill (track 16) **and** on credentials in `.env`, which an agent must not enter. This is the second of the two locks on the Gate 15 clock | **DONE-3, the only calendar-bound condition** | **withdrawn 2026-10-02 (ADR-0048)** |
| **H-4** | **Q-4: capital and liability.** Whose broker, whose money, who is liable when a friend's strategy loses | **all of Stage 6** | **open since inception** |
| ~~H-5~~ | Whether Set C and Set F are retired or re-derived | — | **closed 2026-09-15 without needing you.** The audit answered both from evidence: Set F is 9/11 shipped with 2 deliberately reversed; Set C is 54/68 shipped with 2 open items worth doing |
| **H-6** | Whether equities/futures onboarding is coming, which turns several vacuous invariants live (INV-03, INV-07, INV-08, corporate actions) | Set P scope beyond crypto | open |

**H-2 is now the only credential gate blocking work** — minutes of human action, not
engineering. H-7 was withdrawn on 2026-10-02 (ADR-0048: paper trading runs in the app). H-1 is closed.

---

## 9. Upstream gaps — what must be specified before a set can be planned

Per `add-plan.md`: a formal plan with no source is a guess. These sets **cannot be written
yet**, and this is the list of what to write first.

| # | Gap | Blocks | Fix |
|---|---|---|---|
| G-1 | ~~No spec for the regime engine~~ — **corrected 2026-09-15: this was wrong.** SPEC §5.4 specifies it fully (causal/research GRANT split, two layers, statistical labels) and the rule tier is **built and running** (`regime_jobs`, 21-day vol terciles, `rule_vol_tercile_v1`). The real gaps are narrower: `regime_scopes` has **no default and is unconfigured** (ADR-P3-07), so Gate 11 and Gate 6's regime count cannot compute; the **OOS regime-conditional separation test has never run**, and SPEC §5.4 says without it the regime model *is decorative*; the learned tier (HMM/HSMM + BOCPD) is deliberately deferred by ADR-P4-01 | Gates 6 and 11, Stage 2 | **dispatched as track 12** — configure the scopes and run the separation test; no new spec needed |
| ~~G-2~~ | **Skills lifecycle** — recorded as "no build spec". **Corrected 2026-09-15: too pessimistic, the same error made about G-1.** AGENT-003 §6.2–§6.4 already assign work to R-a and R-b by section, §9 maps every `KM-`/`SK-` requirement, and §10's acceptance is labelled by plan set | Set R-a, Stage 5 | **closed.** AGENT-003 carries a dated amendment note (five items: migration 0039 is taken, §2 and §5 are superseded by AGENT-007, §4/§7 are partly built by tracks 8 and 13, and the pack uses pgvector-exact-kNN not Milvus) and is marked `Derivation Status: STALE`. Set R-a is plannable |
| G-1b | The **insight store** has shapes and grants but no retrieval endpoint, no 4K-token cap, no decay-on-contradiction and no embedder (checklist 3.10). SPEC §5.6 says stale beliefs are *the dominant long-horizon memory failure* and `contradicted_n`/`decay_score` exist to catch them — unread, they catch nothing | the knowledge base compounding at all; Stage 2 | **dispatched as track 13** — specified in SPEC §5.6; no new spec needed |
| G-3 | **Set M residual** — FEAT-005 predates the ledger, the fee book, exits and the oracle test | Stage 4 | **closed 2026-09-15**: FEAT-005 carries a dated amendment note listing the six things that changed underneath it and is marked `Derivation Status: STALE`. Set M is now plannable by re-deriving against that list |
| G-4 | **Set N** — FEAT-006 and Set H Phases 4–5 were written against the pre-`mlops` registry, and `promote_model_version` was ungated | Stage 4 | **closed 2026-09-15**: FEAT-006 carries a dated amendment note (seven items, including promotion becoming a ring-1 proposal) and is marked `Derivation Status: STALE`. Set N is now plannable |
| ~~G-5~~ | **Set F** assumed MCP is the agent's runtime | — | **closed 2026-09-15.** Not stale — mostly already built and never ticked. Set F is closed as superseded by ADR-0022 + AGENT-006; nothing is owed |
| ~~G-6~~ | **Set C** latency: completion never tracked | — | **closed 2026-09-15.** The framing was wrong in a useful way: completion *was* tracked, in `SUMMARY.md`/`SESSION_FINAL_SUMMARY.md`, and never propagated into `MASTER.md`. One sibling record was also wrong (recorded #2 as blocked on rkyv; rkyv shipped) |
| G-7 | `docs/plans/README.md` index lists Set A phases as "Pending" when Set B completed them | navigation | corrected in this pass |
| G-8 | No spec ties **`data_qc` grades to gate eligibility** end to end (DA-07 landed in Set L Phase 6 but the contract is thin) | Set P Phase 2 | extend DATA-005 §7 during P-2 |

Sets **P** and **T** have no gap — DATA-005 §5/§7/§11, 06_DATA §6 S1–S7, `DA-10…DA-12`,
COMP-002, COMP-007, ADR-P2-14, SPEC §12.3 and the capital-ramp migration (0053) are
sufficient, which is why they are written now and the rest are not.

---

## 10. Retirement checkpoints

Carried from `17_BUILD_ORDER §6` and extended. A thing is retired when its replacement has
passed its acceptance, never before.

| When | Retire |
|---|---|
| Set S Phase 2 exit | `crates/api/src/agent/driver.rs` and its prompt module — one loop, every tier (D-S1) |
| Set S Phase 6 exit | the `/agent` and `/research` pages; the deprecated Tailwind aliases (S-6.22) |
| Set S Phase 8 exit | the four hand-maintained tool arrays; `wait_for_backtest` |
| Set T Phase 1 exit | the legacy `POST /api/backtests` route — OQ-08 option A, "when the frontend is next touched" |
| Set M-res exit | the stateless `evaluate_signals` backtest path; `features_compute.rs` |
| Set O-res exit | v1 JSON as the primary authoring format (the translator stays) |
| Stage 5 exit | FEAT-003's original campaign/regime code paths, replaced inside the new harness |

---

## 11. Programme risks

Distinct from per-set risks, which live in each MASTER.

| Risk | Why it is a programme risk | Mitigation |
|---|---|---|
| **Forward-collect series never switched on** | unrecoverable at any cost; compounds daily | Stage 0 is defined so that this is its first task, ahead of all engineering |
| **The evidence clock is discovered late** | finding in 2027 that nothing can pass Gate 9 | measure coverage in P-0 *before* backfill, publish it, and let `paper_v1` carry the interim |
| **Seven concurrent branches collide** | two databases, one Cargo workspace | reserved migration ranges (§7), named shared files, one branch per phase |
| **A downstream set gets planned from a stale spec** | plans that look derived but are not | §9 is the gate; `Derivation Status: STALE` fires automatically via `add-spec.md` step 5 |
| **Scope grows because the design is enjoyable** | thirteen sets and 189 tasks is already the largest thing here | every new capability enters through §9 as an upstream gap first; a set with no spec does not get planned |
| **The agent becomes good at research nobody can act on** | DONE-1 without DONE-3 | Set T runs from Stage 0, not after Set S |
| **Free-only data bounds the ceiling permanently** | ADR-0027 is a policy, not a law of nature | revisit trigger: the first time a gate is unreachable *only* because of source depth, that becomes a costed decision for the user, not a silent limit |
| **`N_eff` grows faster than evidence** | every look deflates; a busy agent makes its own gates harder | this is *correct behaviour*, and the mitigation is yield accounting (DONE-2), not relaxation |

---

## 12. What to do next, concretely

In order, starting now:

1. ~~Answer H-1~~ **— done, `37faade`.** Answer **H-2** (a model-provider credential).
   ~~**H-7** (Alpaca paper credentials)~~ was withdrawn on 2026-10-02: ADR-0048 runs paper
   trading in the app's own engine, so the paper clock needs no credential from you.
2. **Run Set P Phase 0** — measure coverage, publish it, switch on the forward collectors.
   Days, not weeks. This is the irreversible one.
3. **Run Set T Phase 0** — wire reconciliation, deploy one hand-authored strategy to paper.
   The Gate 15 clock starts the day this lands.
4. **Branch Set S Phase 0**, then fan out the five tracks.
5. Re-read §9 before planning anything in Stage 4 or 5. Those sets are not ready and
   pretending otherwise is the one failure this roadmap is built to prevent.

---

## 13. Progress log

| Date | Note |
|---|---|
| 2026-09-15 | **H-1 closed — `37faade`, 161 files, +27,518.** The gate this roadmap called "the highest-value thing you can do" is shut. What made it urgent was not the code: it was that **the plans themselves were untracked too**, so twenty-five worktrees cut from `62e0165` lacked both the mechanisms their tasks depend on and the documents describing the tasks. Every track is now unblocked by a rebase. The mirrored "user's work" commits some tracks made should be **dropped, not deduplicated** — the real commit supersedes them. Two things found while closing it: a **stale `ledger` rlib** made `cargo build -p api` fail with four unresolved-import errors against modules that were declared and present (touch `crates/ledger/src/lib.rs` if it recurs — it is not a real breakage), and **`docs/specs/README.MD` and `README.md` are both indexed** while Windows can hold only one on disk, so that path shows permanently dirty in every worktree. The content is safe under `README.md`; the `.MD` entry is stale. Left alone under §2.6 rather than chased mid-programme, because removing an index entry touches a shared file across twenty-six branches. |
| 2026-09-15 | **G-2 closed, and it was wrong in the same direction as G-1.** I had recorded the skills lifecycle as having no build spec; AGENT-003 already assigns R-a and R-b work by section with a requirements map and plan-set-labelled acceptance. What it needed was an amendment, not a rewrite. **Every one of the eight upstream gaps has now been examined and three of them — G-1, G-2, G-6 — turned out to be pessimistic records of things that already existed.** That is a pattern worth stating: an unverified claim in these plans skews toward "not built" as readily as toward "built", and both directions cost. |
| 2026-09-15 | **Track 1 landed and Stage 1 is genuinely open.** The three contracts five phases waited on exist. H-1 is now a precise ask rather than an audit: `Cargo.toml`'s whole diff is two workspace-member lines, and the 13 untracked catalogue tools cannot compile without them. Its most instructive find is one to remember when reading any green suite here: **`static_invariants.rs:632` asserted the defect** — it checked an action string no tool answers to, so it certified promotion as gated for precisely as long as promotion was ungated. A test can be the thing hiding the hole. |
| 2026-09-15 | **Track 5 landed.** The forward-evidence pipeline exists end to end and has never seen a fill. Gate 15 now leads its verdict with "n observations over d days", the observed-knowledge filter lives in exactly one function, and a planted backfilled row inside the window does not enter the count. The divergence halt sits **ahead of the idempotency cache**, so a redelivered intent approved before a divergence cannot ride the cached-approval fast path. `Numeraire` is a closed enum with **no conversion API** — only `assert_comparable`, which refuses — so USDT cannot be summed as USD by any route. Two honest omissions reported rather than glossed: T-0.3's UI surface was left unbuilt rather than half-built, and the SQL writers are untested because there are no fills to write. **New human gate H-7: Alpaca paper credentials.** Also: it corrected a build instruction I had given every agent — see the board. |
| 2026-09-15 | **Track 3 landed and the evidence clock stopped being a belief.** §1 rewritten against the census rather than an anecdote: 158,259 rows, best continuous run 88 days, **nothing meets `paper_v1` and nothing is within 1,738 days of `strict_v1`** — paper around June 2027 and strict around August 2031 on forward collection alone. Two findings outrank the depth. **First, every row's `knowledge_time` is false** (`available_time = event_time` on all 158,259, REST backfills included), so no existing bar is Gate-15 admissible and INV-01 is violated store-wide — track 20 dispatched. **Second, `market_bar`, the table `BarStore` reads, does not exist in the live database**, so every canonical read returns nothing; the census had to use direct SQL. The forward clock did start: Kraken Futures verified usable from here and collecting 279 perps with funding and OI. Deribit is `BLOCKED:UPSTREAM` — its ToS bars US persons with no market-data carve-out, which is exactly the "verify terms, never build against them" rule working. |
| 2026-09-15 | **Track 15 landed, and found two defects larger than the one it was sent for.** Exits now fire in the simulator, with eleven hand-computed tests and a conservative intrabar rule (`conservative_exit_fill` takes the worse of the resting level and what the market order actually paid, recorded as `integrity_flags` on **every** run — an assumption recorded only when it changes the answer cannot be checked on the runs where it didn't). But: **every fill was charged zero commission**, so every backtest this platform has produced reported a gross number labelled net; and every `Trade` carried size zero. Both affect every run. Also recorded: **the oracle test does not guard the fill path** — it is pure arithmetic in `costs.rs` and never runs the simulator, so it would not have caught this and will not catch a fill-timing regression. I had been treating it as broader protection than it is. And exits now fire in replay but not live, which is an SC-3 violation dispatched as track 19. |
| 2026-09-15 | **Track 11's audit landed and corrected this roadmap.** Four plan-set states in §2 were wrong (F was 9/11 shipped not 0/11; C was 54/68 not untracked; G was executed the day it was written; E is 6/30 not 5/30). G-5, G-6 and H-5 closed from evidence rather than by decision. **And §6's claim that a hand-authored strategy could reach paper "in a day" was false** — there is no working path to a fill at all, which moves Set E's execution items from Stage 3 to Stage 0 and onto the critical path to DONE-3. Dispatched as track 16, ahead of everything. Also surfaced and unassigned-until-now: the kill switch does not survive a restart (sent to track 2), and `hot_path.rs` gates orders against a fabricated risk context, which would have made the single chokepoint a rubber stamp the moment stage 3 was wired. |
| 2026-09-15 | **G-3 and G-4 closed**, and the exits defect found. Amendment notes written into FEAT-005 and FEAT-006 recording what the ML platform pack changed underneath them, both marked `Derivation Status: STALE` — Stage 4's two sets are plannable again. While reconciling, found that the backtest simulator does not call `ExitTracker` although exits are typed fields at `definition_version` 1.3, so **every backtest of an exit-declaring strategy has been evaluating a different strategy than the one declared**. Dispatched as track 15 ahead of all Stage-4 work: a silent correctness failure in the component whose job is to tell the truth outranks new capability. |
| 2026-09-15 | **G-1 corrected.** The regime engine was recorded as unspecified; it is not. SPEC §5.4 specifies it and the rule tier runs. The real gaps are an unconfigured `regime_scopes`, an OOS separation test that has never run, and a learned tier deferred by decision. G-1b added for the insight store. Both dispatched rather than deferred to Stage 5 — they belong to Stage 2, because a gate that cannot compute is a hole in the evidence surface, not a capability wish. |
| 2026-09-15 | Roadmap created. North Star recorded in `artifact.md` (SC-8…SC-13, FM-7…FM-11). Reconciled thirteen plan sets, the ML platform pack and the BS-007 reserved letters into six stages. Two new sets written: **P** (data reach) and **T** (forward evidence), both derived from existing specs and both placed *ahead of* Set S in the evidence order. Supersedes `17_BUILD_ORDER §2–§3`. Eight upstream gaps recorded rather than planned around; six human decision gates named, two of which block this week. |
