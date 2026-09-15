---
Type: Formal
Status: Current
Created: 2026-09-15
Derived From: docs/artifact.md (North Star, SC-8…SC-13, FM-7…FM-11), BS-007 17_BUILD_ORDER §2–§3
  (superseded here), every plan-set MASTER, ml-platform-context-pack HANDOFF/PROGRESS/CURRENT-STATE,
  AGENT-005…014, COMP-007, DATA-005, ADR-0027, ADR-0034…0037, ADR-P2-14
---

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

- **History depth.** `strict_v1` Gate 9 has a five-year calendar floor. Free-only sources
  (ADR-0027) bound how deep a backfill can go, and how deep is *not yet measured* — the
  last recorded check found BTC-USD holding **22 one-hour bars** (DATA-005 S1, 2026-09-08).
- **Forward observation.** Gate 15 takes evidence only from observed-knowledge rows. A
  forward test cannot be backfilled, accelerated, or bought. It is a calendar.
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

## 2. Where we actually are (2026-09-15)

Verified against the repository, not against the plan files' own claims.

### Built and load-bearing

| Area | State | Evidence |
|---|---|---|
| Rust platform | 43 crates, serves on 7080, Postgres + ClickHouse + NATS + Redis | `crates/`, `config/local.toml` |
| Migrations | **at 0060**, developer database at 59 | `migrations/`, HANDOFF §5b |
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
| **Campaign child workers missing** (`Study`, `EvalTask`, `ResearchRun`) | a campaign fails at its first dispatching phase with `no_worker`; nothing consumes `Comparison` | HANDOFF §5.2 — **Set S Phase 10 supplies `ResearchRun`** |
| **5 of 16 gates have no evidence source** (3, 4, 10, 12, 15) | the funnel cannot render a complete verdict | HANDOFF §5.3 |
| **Gate 15 has no input at all** | the reconciliation crate is not wired — so *no forward evidence exists* | HANDOFF §5.3 → **Set T** |
| **AT-61 unwritten** | no framework can claim `bit_identical` resume, so every resume restarts under the same trial | HANDOFF §5.1 |
| **Phase 3 writers unscheduled** | `asset_embedding` and the outcome tensor have code and tests but nothing writes them | HANDOFF §5.4 |
| **The external MCP front door writes no audit trail** | a named exemption, not an oversight, but a real hole | AT-64 |
| **Data depth unmeasured and believed thin** | gates the entire evidence clock | DATA-005 S1 → **Set P** |
| Four `storage` live suites + `lint-no-json-hotpath` fail | pre-existing; documented do-not-chase | HANDOFF §3 |

### Plan sets, honestly

| Set | Subject | State | Disposition |
|---|---|---|---|
| A | Python→Rust rewrite (original) | superseded | closed by B |
| B | Full end-state platform | **Complete** | closed |
| C | Latency optimisation (68 issues) | **status never tracked to completion** | audit before reuse — §9 gap G-6 |
| D | Backtest decoupling/hardening | 31/33 (94%), 2 deferred | closed, deferred items recorded |
| E | Production readiness / tech debt | **5/30 (17%)** | live backlog — folded into Stage 3 |
| F | MCP platform expansion | 0/11 | **re-derive or retire** — written before ADR-0022 made MCP a thin client; §9 gap G-5 |
| G | Documentation restructuring | 0/11 | largely overtaken by events; audit and retire |
| H | AI Model Studio | 47/64 (73%), Phases 4–5 pending | folded into Set N |
| I | Probabilistic forecasting | **77/77 Complete** | closed |
| J | Honest-evaluation core | **59/59 Complete** | closed |
| K | Make backtest real | superseded | closed by L |
| L | Agent foundation | Phases 0,1,2,5,6 core complete; 3–4 built, never run live | closed; Set S rebuilds on its evidence |
| **S** | **The Agent Platform** | **0/189 — NOT STARTED, ready** | **Stage 1** |
| **P** | **Data reach** | **new, written 2026-09-15** | **Stage 0 — starts immediately** |
| **T** | **Forward evidence** | **new, written 2026-09-15** | **Stage 0/2 — starts immediately** |
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
Stage 3  │ PAPER AT SCALE    Set T Ph.3-4 · Set E · COMP-007 live · ramp     │
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

Set T Phases 3–4 (deployment proposals, capital-ramp mechanics, the paper fleet), COMP-007
promoted from "built in Set S Phase 7" to "running in production with drills", and Set E's
remaining 25 production-readiness items, which become load-bearing the moment something
runs unattended against a broker.

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

| Letter | Original scope | Taken by Set S | **Residual scope** | Stage |
|---|---|---|---|---|
| **M** Engine truthfulness | `FE-`, `BT-`, `ST-` Layer 1 | the DATA-006 F1/F2 feature engine and the one-engine rule (Phase 5) | backtest-engine truthfulness: `BT-01…BT-11` beyond features, fill realism, the `market_simulator` contract | 4 |
| **N** Models | `MD-01…MD-13`, TB §3.7 | nothing | whole, **plus Set H Phases 4–5** (inference service, studio frontend) | 4 |
| **O** Strategy Language v2 | `ST-05…ST-12`, `UI-09` | SLv2 as the model-facing language, compiler, moves (Phase 5) | the **visual builder's** v2 surface, `UI-09` | 4 |
| **P** Data reach | `DA-10…DA-12`, S1–S7 | nothing | **whole — written 2026-09-15, [`set-P/MASTER.md`](plan-sets/set-P/MASTER.md)** | **0–1** |
| **Q** Research standard | `EV-01`, `EV-03…EV-08` | the honesty arithmetic and the leakage battery (Phases 3, 8) | remaining `EV-` extensions; 11 §8 High tools not in the Set S catalogue | 2 |
| **R-a** Skills lifecycle | `SK-02…SK-08, 10, 12…15`, `UI-06` | playbook admission, episodic + semantic memory (Phase 4) | the registry, materialiser, propose/verify, project admission, `UI-06` | 5 |
| **R-b** Knowledge and autonomy | `KM-02…KM-10`, `SK-11/16/17`, `EV-09` | campaigns as the `optimise` workflow; autonomy as continuous research (Phases 10–11) | the **regime engine**, knowledge promotion, curator, A/B, random arm, R1 techniques | 5 |
| **T** Forward evidence | was "Track (own set)" | nothing | **whole — written 2026-09-15, [`set-T/MASTER.md`](plan-sets/set-T/MASTER.md)** | **0–3** |

Letters already used and closed: A, B, C, D, E, F, G, H, I, J, K, L, S. The next free
letter after T is **U**.

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

1. **The agent is not on the critical path to first evidence.** A hand-authored strategy
   deployed to paper in Stage 0 starts the Gate 15 clock just as well as an agent-authored
   one, and costs a day. Do that. The agent's value is throughput and honesty at scale,
   which matters from Stage 2 onward.
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
| Stage 4 (M-res, N, O) | 0121–0140 | reserved |
| Stage 5 (R-a, R-b) | 0141–0160 | reserved |

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
| **H-1** | The dirty working tree: which of the ~50 modified files are yours and must not be committed? | every Set S branch | **open — Stage 0, task S-0.1** |
| **H-2** | A model provider credential in the encrypted store | 2 Set S acceptance criteria; every live agent session; all of Stage 1's measurement | **open — Stage 0, task S-4.14** |
| **H-3** | Monthly research budget ceiling (the platform default is $25/day per continuous session) | continuous research at scale | default set, revisit at Stage 2 |
| **H-4** | **Q-4: capital and liability.** Whose broker, whose money, who is liable when a friend's strategy loses | **all of Stage 6** | **open since inception** |
| **H-5** | Whether Set C (latency) and Set F (MCP expansion) are retired or re-derived | roadmap hygiene | open — §9 |
| **H-6** | Whether equities/futures onboarding is coming, which turns several vacuous invariants live (INV-03, INV-07, INV-08, corporate actions) | Set P scope beyond crypto | open |

**H-1 and H-2 are the only two that block work starting this week.** Both are minutes of
human action, not engineering.

---

## 9. Upstream gaps — what must be specified before a set can be planned

Per `add-plan.md`: a formal plan with no source is a guess. These sets **cannot be written
yet**, and this is the list of what to write first.

| # | Gap | Blocks | Fix |
|---|---|---|---|
| G-1 | ~~No spec for the regime engine~~ — **corrected 2026-09-15: this was wrong.** SPEC §5.4 specifies it fully (causal/research GRANT split, two layers, statistical labels) and the rule tier is **built and running** (`regime_jobs`, 21-day vol terciles, `rule_vol_tercile_v1`). The real gaps are narrower: `regime_scopes` has **no default and is unconfigured** (ADR-P3-07), so Gate 11 and Gate 6's regime count cannot compute; the **OOS regime-conditional separation test has never run**, and SPEC §5.4 says without it the regime model *is decorative*; the learned tier (HMM/HSMM + BOCPD) is deliberately deferred by ADR-P4-01 | Gates 6 and 11, Stage 2 | **dispatched as track 12** — configure the scopes and run the separation test; no new spec needed |
| G-2 | No spec for the **skills lifecycle** at implementation fidelity (AGENT-003 is a Phase-0 contract; BS-007 13 is spec-ready but not a build spec) | Set R-a, Stage 5 | re-derive AGENT-003 §skills into a build spec before planning Set R-a |
| G-1b | The **insight store** has shapes and grants but no retrieval endpoint, no 4K-token cap, no decay-on-contradiction and no embedder (checklist 3.10). SPEC §5.6 says stale beliefs are *the dominant long-horizon memory failure* and `contradicted_n`/`decay_score` exist to catch them — unread, they catch nothing | the knowledge base compounding at all; Stage 2 | **dispatched as track 13** — specified in SPEC §5.6; no new spec needed |
| G-3 | **Set M residual** — FEAT-005 predates the ledger, the fee book, exits and the oracle test | Stage 4 | **closed 2026-09-15**: FEAT-005 carries a dated amendment note listing the six things that changed underneath it and is marked `Derivation Status: STALE`. Set M is now plannable by re-deriving against that list |
| G-4 | **Set N** — FEAT-006 and Set H Phases 4–5 were written against the pre-`mlops` registry, and `promote_model_version` was ungated | Stage 4 | **closed 2026-09-15**: FEAT-006 carries a dated amendment note (seven items, including promotion becoming a ring-1 proposal) and is marked `Derivation Status: STALE`. Set N is now plannable |
| G-5 | **Set F** assumes MCP is the agent's runtime; ADR-0022 made it a thin client and ADR-0035 demoted the container | Set F | retire Set F, or re-derive its 11 tasks against AGENT-006 |
| G-6 | **Set C** latency: 68 issues, completion never tracked | reuse | audit against current code, then close or re-cut |
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

1. **Answer H-1** (the dirty tree) and **H-2** (the credential). Nothing else is blocked on
   anything you have to decide.
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
| 2026-09-15 | **G-3 and G-4 closed**, and the exits defect found. Amendment notes written into FEAT-005 and FEAT-006 recording what the ML platform pack changed underneath them, both marked `Derivation Status: STALE` — Stage 4's two sets are plannable again. While reconciling, found that the backtest simulator does not call `ExitTracker` although exits are typed fields at `definition_version` 1.3, so **every backtest of an exit-declaring strategy has been evaluating a different strategy than the one declared**. Dispatched as track 15 ahead of all Stage-4 work: a silent correctness failure in the component whose job is to tell the truth outranks new capability. |
| 2026-09-15 | **G-1 corrected.** The regime engine was recorded as unspecified; it is not. SPEC §5.4 specifies it and the rule tier runs. The real gaps are an unconfigured `regime_scopes`, an OOS separation test that has never run, and a learned tier deferred by decision. G-1b added for the insight store. Both dispatched rather than deferred to Stage 5 — they belong to Stage 2, because a gate that cannot compute is a hole in the evidence surface, not a capability wish. |
| 2026-09-15 | Roadmap created. North Star recorded in `artifact.md` (SC-8…SC-13, FM-7…FM-11). Reconciled thirteen plan sets, the ML platform pack and the BS-007 reserved letters into six stages. Two new sets written: **P** (data reach) and **T** (forward evidence), both derived from existing specs and both placed *ahead of* Set S in the evidence order. Supersedes `17_BUILD_ORDER §2–§3`. Eight upstream gaps recorded rather than planned around; six human decision gates named, two of which block this week. |
