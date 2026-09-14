# Quant Research Agent — Agent Foundation — Set L

**Completion: Phase 0 complete; Phases 1, 2, 5 and 6 core complete; Phases 3–4 built
but never run live. See §4 for the honest per-phase state.**

**The one thing that has never happened: a live agent session.** It needs an Anthropic
credential in the encrypted store, which this work did not have and did not create.
Everything either side of that gap is built and verified — the sandbox holds, tokens
mint with the right scopes, `tbot` works against the live platform, the workspace
streams a real timeline, and the eval harness grades and scores without a model. The
suites that need a session (`noise`, `planted_edge`) are written and dry-runnable; the
auditor suite, which needs no agent at all, runs in CI at 100% catch and 0 false
rejections.

**Status:** IN PROGRESS (2026-09-11). Phase 0 was pulled forward and was the only phase
that could run before anything else landed — it closes a live data-loss bug and it blocks the bar
backfill. Set L then builds the agent foundation: the Agent SDK runtime in a
per-project container, the LLM proxy, the durable job service and artifact store, the
Data API v2 with the research cutoff, the `tbot` read surface, the workspace UI v1, and
the eval-suite skeleton. It also absorbs the two unshipped legs of Set K (durable
stores, parallel execution).

**Created:** 2026-09-11
**Builds on:** Set J (`docs/plans/plan-sets/set-J/MASTER.md`), Set K
(`docs/plans/plan-sets/set-K/MASTER.md` — see §6)
**Derived from:** BS-007
[00_INDEX](../../../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/00_INDEX.MD),
[17_BUILD_ORDER §2](../../../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/17_BUILD_ORDER.MD)
**Formal specs:** [AGENT-001](../../../specs/AGENT-001-agent-runtime.md),
[AGENT-002](../../../specs/AGENT-002-agent-toolbox.md),
[AGENT-003](../../../specs/AGENT-003-knowledge-memory-skills.md),
[AGENT-004](../../../specs/AGENT-004-agent-evaluation-suite.md),
[COMP-005](../../../specs/COMP-005-job-service-and-artifacts.md),
[COMP-006](../../../specs/COMP-006-research-workspace-ui.md),
[DATA-005](../../../specs/DATA-005-data-api-v2.md)
**ADRs:** 0024, 0025, 0028, 0030 (all Accepted 2026-09-11)
**Scope class:** End-state architecture — every subsystem is specified at full
fidelity; phases are build-ordering, not feature-gating (the Set J convention).

---

## 1. Overview

Set L makes the platform agent-first. It delivers the runtime the quant research agent
lives in, and the platform services that make the agent's honesty structural rather
than prompted.

The load-bearing idea, from ADR-0025: **an agent with bash and code in a sandbox can
compute anything on any data it can read, and it can ignore any prompt.** So the
research invariants are not rules the agent is asked to follow. They are properties of
the services it must go through:

- it cannot see past the research cutoff, because the Data API will not return that
  data to a research token (DA-02…DA-05);
- it cannot avoid counting a trial, because the count happens at job submission
  (COMP-005, INV-1);
- it cannot grant itself authority, because the session token carries scopes it cannot
  mint (RT §6);
- it cannot publish an unsupported claim, because `final_report` is validated
  server-side (RT §15).

**What this set delivers:**

- **Phase 0 — `market_bars_v2`.** An append-only bar table with `timeframe` in the
  sorting key, a verified row-for-row cutover, and writers moved onto it. **(DA-16,
  DA-17)**
- **Phase 1 — Jobs and artifacts.** One durable, idempotent job service and a
  content-addressed artifact store; trials counted at submission; the Set J suite moved
  off in-memory stores onto durable ones. **(JB-01…JB-12; Set K B, C)**
- **Phase 2 — Data API v2.** PIT reads, the research cutoff, the Desk project, token
  scopes, `data_qc` grades, the exploration ledger. **(DA-01…DA-09, DA-13, DA-15)**
- **Phase 3 — LLM proxy and the container.** `crates/llm-proxy` with credential
  injection, budgets, usage telemetry and the `cache_control.ttl` rewrite; the
  `tbot-agent` image; the rootless sandbox with no egress but the platform.
  **(RT-01…RT-09, RT-22, RT-23; CX-01…CX-14)**
- **Phase 4 — Orchestrator and `agent-host`.** Project and session lifecycle, the event
  bridge, hooks, sub-agents, steering, `ask_user`, resume. **(RT-10…RT-21)**
- **Phase 5 — `tbot` read surface and core skills.** The SDK/CLI the agent reaches the
  platform through, plus the static core skill set. **(TB §3.1–3.3, §3.9 read, §3.10;
  SK-01, SK-09; KM-01)**
- **Phase 6 — Workspace UI v1 and the eval skeleton.** The human viewer over the same
  APIs, and the eval harness that gates every later phase exit. **(UI-01…UI-05, UI-07,
  UI-08; AE-01…AE-05; EV-02, EV-10)**

**Why Phase 0 is first and separate.** It is not agent work. It is a live data-loss
bug that was measured on the production table on 2026-09-11 (DATA-005 §3): `market_bars`
is `ReplacingMergeTree(revision) ORDER BY (instrument_id, available_time)` with
`timeframe` absent from the key, so a 1m and a 1h bar that close at the same instant are
treated as duplicates and one is destroyed on merge. 18 of BTC-USD's 22 1h bars — every
1h bar overlapping 1m coverage — were queued for destruction. They were rescued to a
snapshot before any merge, so nothing is lost today, but two facts force this to the
front of the set:

1. **The bleed continues.** Every new coarse bar landing on a boundary where a fine bar
   exists creates a fresh collision, until writers move to v2.
2. **The backfill would make it much worse.** The deep-history ops task (DATA-005 §11
   S1) writes exactly the bars that collide. **The v2 cutover must land before the
   backfill runs.** This ordering constraint did not exist in 17_BUILD_ORDER and is
   recorded here.

---

## 2. Scope

### In scope

| Area | Today | Set L delivers |
|---|---|---|
| Bar storage | `ReplacingMergeTree`, `timeframe` not in the key; cross-timeframe collapse | append-only `market_bars_v2`, revisions retained, verified cutover |
| Long work | ad-hoc: backtest manager with a fixed 3-concurrent cap, sweeps in memory, asset-init jobs, training runs | one durable job service, idempotent, restart-safe, with a content-addressed artifact store |
| Trial counting | in-memory (`InMemory*Store`); does not survive a restart | counted at job submission, in Postgres, monotonic |
| Data access | direct store reads, no cutoff concept | Data API v2: PIT, cutoff, Desk, scopes, `data_qc`, exploration ledger |
| Agent runtime | a Rust loop resending the whole transcript over 38 MCP tools, no caching, no resume, no files | Claude Agent SDK in a per-project rootless container, behind an LLM proxy |
| Model credentials | none for the agent | proxy-injected; the container holds no secret |
| Agent tool surface | MCP tools only | `tbot` SDK/CLI over `/api/*`, harness-neutral |
| Human UI | `/agent` page over the old loop | research workspace v1 over the same APIs |
| Agent evaluation | none | eval suite skeleton, incl. the pure-noise task, gating phase exits |

### Out of scope (this set)

- **The feature engine and backtest truthfulness** — Set M (FE-, BT-, ST- Layer 1).
  Set L consumes the engine as it stands.
- **The research standard extensions** — Set Q (EV-01, EV-03…EV-08). Set L carries only
  EV-02 and EV-10.
- **Models and prediction series** — Set N (MD-).
- **Strategy Language v2** — Set O (ST-05…ST-12).
- **The skills lifecycle** — Set R-a. Set L ships *static* core skills (SK-01, SK-09)
  with no registry, proposal or admission flow.
- **Knowledge and autonomy** — Set R-b (KM-02…KM-10, campaigns, regime engine).
- **New data sources** — Set P (DA-10…DA-12, S1–S7). Phase 0 fixes the *schema* the
  backfill will write into; it does not add sources.
- **Trading, automation, alias promotion, holdout access for the agent.** Never in
  scope, in any set (D-12, ADR-0025).

---

## 3. Locked decisions carried into this set

| ID | Decision | Source |
|---|---|---|
| D-01 | Frontier models first | 00_INDEX |
| D-02 | One sandboxed container per research project | ADR-0024 |
| D-07 | Harness = Claude Agent SDK, pinned at `claude-agent-sdk==0.2.152` | ADR-0024, AGENT-001 §23 |
| D-10 | Invariants enforced at platform services, not prompts | ADR-0025 |
| D-11 | Never trade model capability for tokens; remove waste instead | 04_CONTEXT_AND_COST |
| D-12 | Research authority only — no orders, no automations, no live arming | ADR-0025 |
| D-13 | Exploration is logged, not counted as trials | 11 §3 |
| D-15 | Desk project: cutoff = now, exploratory only, no G3 and no vault | ADR-0025 |
| D-16 | Billing: Anthropic API key primary, Claude subscription fallback; the proxy selects per project and pins the kind per session | ADR-0024 §3 |
| D-17 | A local GPU is available; it belongs to the training-job worker, not the agent container | 00_INDEX |
| INV-1/2/3, P1 | Set J invariants and ADR-0023 P1 hold everywhere | Set J |

---

## 4. Phase summary

| Phase | Title | Requirement IDs | State |
|---|---|---|---|
| **0** | [Bar schema v2 and verified cutover](phase-0-bar-schema-v2.md) | DA-16, DA-17 | **Complete** (9/9), amended after production use |
| **1** | [Durable job service and artifact store](phase-1-jobs-and-artifacts.md) | JB-01…JB-12; Set K B, C | **Core complete.** Runs end to end on the platform. Set K C deferred |
| **2** | [Data API v2, cutoff, Desk](phase-2-data-api-and-cutoff.md) | DA-01…DA-09, DA-13, DA-15 | **Cutoff core complete and verified.** DA-13 (synthetic venue) and DA-07 (grade gating) landed in Phase 6. Most §5 endpoints not built |
| **3–4** | [LLM proxy, runtime, orchestrator](phase-3-4-runtime-and-orchestrator.md) | RT-01…RT-23; CX partial | **Built and unit-tested; no live agent session has run** (needs a provider credential) |
| **5** | [`tbot` toolbox](phase-5-6-toolbox-ui-and-evals.md) | TB §3.1–3.3, §3.10 | **Read surface working** against the live platform. Skills and memory not built |
| **6** | [Workspace UI and eval suite](phase-5-6-toolbox-ui-and-evals.md) | UI-, AE-, EV-02, EV-10 | **Core complete and verified in a browser.** Auditor suite (AE-02) runs in CI: 27 fixtures, 100% catch, 0 false rejections. Synthetic venue, graders, scorecard and the orchestrator's HTTP surface all work. Live trials blocked on a credential; EV-02/EV-10 untouched |

**The honest summary:** the *enforcement* half of Set L is built and demonstrated —
trials are counted in Postgres inside the submission transaction, the research cutoff
clips every read, scoped tokens cannot reach trading, the holdout or the eval answer
key, and the sandbox holds. The *agent* half is assembled but unproven: every
component exists and is tested, a real timeline streams into a real workspace, and no
agent has yet run a research session end to end.

**One finding that is a decision, not a task.** AGENT-004 §7's 2 pp non-inferiority
margins need ~189 paired trials per arm, which at $6–8 a trial is $1,100–1,500 per
gated pull request; a release run of ~75 decides about 4.9 pp. `evals/gates.yaml` is
set to 5 pp with the target recorded, and the gate reports UNDERPOWERED rather than
FAIL when a run cannot decide its own margin. Reaching 2 pp means either paying for
the trials or moving the gated metrics from binary pass/fail to a continuous score.
See phase-5-6 and `evals/README.md`.

---

## 5. Exit criteria

Set L is done when every requirement ID above meets the acceptance criteria in its
formal spec, and specifically:

1. AGENT-001 §16 acceptance passes, including a session that survives a platform
   restart via SDK resume.
2. 04_CONTEXT_AND_COST §12 items 1–4 pass, with no capability regression on the eval
   suite (non-inferiority, D-11).
3. COMP-005 §9 acceptance passes; the trial counter survives a restart.
4. 06_DATA §10 items 1, 2 and 5 pass; a research token cannot read past the cutoff, and
   the attempt is logged.
5. 16_AGENT_EVALS §10 item 1 passes: the pure-noise task ends in a report that claims no
   edge.
6. No row of `market_bars` is absent from `market_bars_v2` (DA-17 checksum).

---

## 6. Relationship to Set K

Set K (2026-06-18) planned four phases. Re-checked against the code on 2026-09-11:

| Set K phase | State | Disposition |
|---|---|---|
| **A** — real `SimRunExecutor` | **Shipped.** `crates/backtest/src/sim_executor.rs`, injected at `crates/api/src/state.rs:174`. A-3's param gap is closed by `domain::strategy_def::params::materialize` | Closed |
| **B** — Postgres/ClickHouse stores | **Not shipped.** Only `InMemoryRunStore` / `InMemoryStudyStore` / `InMemoryExperimentStore` exist. The trial counter does not survive a restart — a live INV-1 hole | **Folded into Set L Phase 1** |
| **C** — parallel execution | **Not shipped.** No fan-out or bounded scheduler in `suite.rs` | **Folded into Set L Phase 1**, as the job service is the scheduler (ADR-0030) |
| **D** — unified workspace | **Shipped.** `/workbench` and `/proving-ground` redirect to `/backtesting` | Closed |

Set K is therefore superseded: A and D are done, B and C live here. Mark Set K's MASTER
accordingly rather than executing it separately.

---

## 7. Progress log

| Date | Phase | Task | Note |
|---|---|---|---|
| 2026-09-11 | — | plan | Set L created. Phase 0 (`market_bars_v2`) pulled to the front: the collapse bug was confirmed on the live table and the 18 at-risk rows rescued to a snapshot; the v2 cutover must precede the bar backfill. Set K B and C folded into Phase 1. DA-16 and DA-17 added to 06_DATA for the schema fix. |
