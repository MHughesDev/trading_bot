# BS-007 agent handoff: quant research agent (2026-09-11)

This is for the next agent picking up the BS-007 work. Read this first, then
[00_INDEX.MD](00_INDEX.MD).

---

## 1. Where things stand

*Rewritten 2026-09-11 (second pass). The first version of this section said "nothing
is implemented"; that is no longer true.*

- **Branch:** `docs/bs-007-quant-research-agent`, branched from `main` at `9f71f22`.
  **Not pushed, no PR, and nothing since `aefdc3d` is committed.**
- **Do not touch or commit** the user's own uncommitted changes: `Cargo.lock`,
  `Cargo.toml`, `apps/platform/src/main.rs`, `crates/config/Cargo.toml`,
  `crates/config/src/model.rs`. (The local-tier work was deliberately shaped to avoid
  needing a `main.rs` edit — see `AgentManager::new` and `state::load_profiles`.)

### Built

| Area | Where |
|---|---|
| Set L Phase 6 — workspace UI, eval suite skeleton, synthetic venue, auditor | `crates/backtest/src/synthetic.rs`, `crates/api/src/{auditor,leakage_lint,data_admission}.rs`, `evals/`, `frontend/src/components/workspace/` |
| Harness conformance (ADR-0031) — profiles, tool registry + risk taxonomy, validation ladder, context manager, policy, provenance, workspace contract | `crates/harness/`, `crates/mcp-server/src/taxonomy.rs`, `config/profiles/` |
| The local tier (ADR-0032) — the GOVERNOR loop, the canary, the fence, the tool bridge, vLLM | `crates/harness/src/{drive,canary,hardware}.rs`, `crates/api/src/agent/{bridge,local_driver,hardware_probe}.rs`, `crates/llm/` |

### Not done

- The user has not accepted the ADRs; all specs are still **Proposed**.
- The runtime spike has not run, and the backfill ops task has not started.
- **Sets M–R are not started.** This is the bulk of the remaining build.
- The workspace contract exists (`harness::workspace`) but is **not mounted into the
  container**, and `deliver` is not exposed to the agent. Both are ADR-0024 container
  work.
- `search_scratchpad` as a first-class tool is not built.
- **Durable resume of the GOVERNOR loop.** An approval row outlives a restart, but the
  loop state does not, so `recover_orphans` cancels pending approvals rather than
  leaving a question nobody is listening to. That is honest, not finished.
- The **reference** model has not run: the dev box (1080 Ti, 11 GB) cannot hold it,
  which is expected (D-18). The loop itself has run end to end on the `degraded` tier
  — see below and [docs/LOCAL_TIER_FINDINGS.md](../../LOCAL_TIER_FINDINGS.md) §8.

### It has run

The GOVERNOR loop completed a task end to end on 2026-09-12: real hardware probe
(GTX 1080 Ti → `degraded`), real canary (4/4 honoured), plan, deterministic routing,
two-step constrained decode, policy ruling, a real platform tool call, and
`finish_task` with evidence. Status `completed`.

The same session's first attempts found **five bugs that 1310 passing tests did not**,
including one where the harness fenced a healthy session because a reply was cut off at
the output cap, and one where the exposure budget hid the tool the model had just
searched for. All five are fixed with tests.
[docs/LOCAL_TIER_FINDINGS.md](../../LOCAL_TIER_FINDINGS.md) §8 has the detail.

The lesson worth carrying: **run it.** Every one of those bugs lived in the space
between components that were each individually correct.

### The agent is a chat now (ADR-0033)

The run form is gone. The user sends a message; the agent picks the instrument, the
timeframe, how many backtests it needs and when it is done. There is **no wall clock
and no iteration cap** — it runs until `finish_task`, and the stop conditions that
remain are Stop, stuck-detection and the fence.

Also new: a conversation sidebar with a running indicator, a per-conversation folder
with `fs` tools over it, LLM-generated titles, and compaction that only fires at 95%
of the window instead of on every assembly.

### The one thing to understand before touching the local tier

`crates/harness/src/drive.rs` is **sans-IO on purpose** and there is a test that fails
the build if that changes. The reason is in the failure-injection suite: a backend
restarting mid-session, a model evicted from VRAM, guided decoding falling back on
step four, a second GPU that is not actually pooled — each of those is a `Vec<Input>`
and each runs green on a machine with no accelerator. Put a `reqwest` call in there
and that property is gone.

The second thing: **a constraint violation is evidence about the backend, not about
the model.** Under a grammar the sampler cannot emit a forbidden token, so a
non-conforming reply proves the grammar was not applied. `validation::conforms` is
strict where `check_schema` coerces, and the difference is deliberate — repairing
`"500"` to `500` is right for a model's guess and destroys the evidence when it is a
backend's lie. One violation fences the session.

## 2. What BS-007 is (30-second version)

The platform is being made **agent-first**, around one long-running quant research
agent:
- **Runtime:** the Claude Agent SDK (Python) in a sandboxed container per research
  project. It reaches the platform only through `/api/*`, via a `tbot` SDK/CLI. Model
  calls go through a platform LLM proxy, so the container holds no secrets.
- **Enforcement:** research invariants are enforced by **platform services**, not
  prompts:
  - a Data API research cutoff (the holdout);
  - trial counting at job submission;
  - token scopes;
  - a `final_report` validator;
  - skill admission.
- **Supporting pieces:**
  - one feature engine;
  - two-layer strategies (Python research strategies + Strategy Language v2);
  - an honest backtest integration;
  - models via walk-forward prediction series;
  - an extended Set J research standard;
  - agent-created skills admitted by the platform;
  - free data sources only.

## 3. Locked decisions (by the user) — do not reopen without asking

D-01…D-13 in [00_INDEX.MD](00_INDEX.MD). Key ones:
- frontier models first;
- one agent, no modes;
- a container per project;
- **harness = Claude Agent SDK**;
- **two-layer strategies**;
- **free data only**;
- local embeddings;
- the agent creates skills, the platform admits them, core and global skills are used for
  every user, and every admitted skill is in the glossary;
- never trade model capability for tokens;
- research authority only: the agent never trades.

## 4. Map of the documents

| What | Where |
|---|---|
| Index, decisions, glossary, open questions | [00_INDEX.MD](00_INDEX.MD) |
| As-built baseline + gap register G-01…G-14 (audited 2026-09-10; re-verify line refs) | [01_GROUND_TRUTH.MD](01_GROUND_TRUTH.MD) |
| Normative subsystem docs with requirement IDs (RT-, CX-, JB-, DA-, FE-, ST-, BT-, MD-, EV-, KM-, SK-, TB-, UI-, AE-, B-) | 02–16 in this folder |
| Phases → plan sets L…R, requirement-ID scope per set, migration numbers, risks | [17_BUILD_ORDER.MD](17_BUILD_ORDER.MD) |
| Evidence and sources (informative) | [research/](research/) (R4 = token-efficiency audit) |
| Formal specs (the contracts plan sets implement) | `docs/specs/`: AGENT-001…004, COMP-005, COMP-006, DATA-005, DATA-006, FEAT-004…006, BACKTEST_SUITE_CORE_SPEC_V2 |
| ADRs | `docs/adr/0024`–`0030` (index updated in `docs/adr/README.md`) |

**Precedence:** the formal spec in `docs/specs/` is more concrete. The BS-007 subsystem
doc is the rationale. The research docs are evidence only.

## 5. Next steps, in order

> **Progress 2026-09-11 (second session).** Steps 1, 2 and 5 are **done**, and step 3 is
> done for identifiers but not for the end-to-end spike. Details:
> - ADRs 0024–0030 are **Accepted**. The user's answers are locked as **D-14…D-17** in
>   00_INDEX: Desk project as specified; billing = Anthropic API key primary with a
>   Claude subscription fallback; a local GPU is available. **RT-22** and **RT-23** were
>   added to 03_RUNTIME for the proxy consequences.
> - **SDK identifiers verified** against `claude-agent-sdk==0.2.152`. AGENT-001 §23 is
>   rewritten as findings and no `(verify)` markers remain. Two mechanisms changed:
>   there is **no `PostCompact` hook** (use `PreCompact` + `UserPromptSubmit`), and
>   **cache TTL is not an SDK option** (the proxy must rewrite it — RT-22).
> - **`market_bars` bug confirmed on the live table** with read-only queries. Numbers and
>   the operational warning are in DATA-005 §3; G-13 in 01_GROUND_TRUTH now carries the
>   measured history. **18 rows are queued to die on the next merge of partition 202605
>   — do not `OPTIMIZE` `market_bars` before the v2 backfill.**
> - **Set K reconciled against the code:** Phase A (real `SimRunExecutor`) and Phase D
>   (unified workspace) are shipped; **Phase B (Postgres/ClickHouse stores) and Phase C
>   (parallel execution) are not** — the suite still runs on `InMemory*Store`, so the
>   trial counter does not survive a restart. Fold B and C into Set L.
>
> - **The 18 at-risk rows were rescued**, before any merge: table
>   `trading.market_bars_rescue_20260911` (plain `MergeTree`, survives `OPTIMIZE … FINAL`
>   intact) plus verified off-repo exports in `../trading_bot_backups/`. Equality was
>   proven with an order-independent `sipHash64` over all 18 columns, and the `Native`
>   export was restored into a scratch table and re-checksummed against the live table.
> - **Plan set L is written** (`docs/plans/plan-sets/set-L/`): MASTER plus
>   `phase-0-bar-schema-v2.md` in full. Set K is marked superseded. `DA-16` and `DA-17`
>   were added to 06_DATA, because the schema fix had no requirement ID to cite.
>
> **Ordering constraint, new:** the bar backfill is **blocked on Set L Phase 0**. The
> backfill writes exactly the bars that collide, so `market_bars_v2` must land first.
> Recorded in 17_BUILD_ORDER §1.
>
> **Implementation session, same day.** Set L Phase 0 is complete, and Phases 1, 2 and
> 5 have working cores. Phases 3–4 are built but have never run live. Phase 6 is not
> started. The per-phase detail is in `docs/plans/plan-sets/set-L/`; the short version:
>
> | Phase | State |
> |---|---|
> | 0 bar schema v2 | **Complete**, amended twice after production use |
> | 1 jobs and artifacts | **Core complete**, runs end to end on the platform |
> | 2 Data API and cutoff | **Cutoff core complete**; most §5 endpoints absent |
> | 3–4 proxy, runtime, orchestrator | **Built, unit-tested, never run live** |
> | 5 `tbot` toolbox | **Read surface working** against the live platform |
> | 6 UI and eval suite | **Core complete and verified in a browser**; live trials blocked on a credential |
>
> **Phase 6, same day.** The synthetic venue (DA-13, seven seeded generators), the
> auditor suite (AE-02: 27 fixture pairs, in CI, 100% catch and 0 false rejections),
> the graders and the paired non-inferiority gate, the orchestrator's HTTP surface
> (SSE timeline, steering inbox, approvals, usage), and the research workspace UI
> (`/research`, `/approvals`). Four guards the auditor fixtures needed did not exist
> and were built: a static leakage lint, a skill tuned-constant lint, DA-07 grade
> gating, and the revised-data pinning check. Migration `0039` adds
> `synthetic_instruments`, `eval_runs`, `eval_scorecards` and `session_inbox`, and
> forbids a new scope, `evals.truth`, on any project-bound session — an agent that can
> read the planted mechanism does not have to find it.
>
> **A decision the next session should put to the user.** AGENT-004 §7's 2 pp
> non-inferiority margins need ~189 *paired* trials per arm (Tango's score interval at
> zero discordance). A release run of ~75 decides about 4.9 pp; at $6–8 a trial a 2 pp
> gate costs $1,100–1,500 per gated PR. `evals/gates.yaml` is set to 5 pp with
> `spec_margin: 0.02` recording the target, and the gate reports **UNDERPOWERED**
> rather than FAIL when a run cannot decide its own margin. Reaching 2 pp means either
> paying for the trials or moving the gated metrics from binary pass/fail to a
> continuous score — the latter is the better trade and touches every grader.
>
> **The enforcement half of Set L is real and demonstrated.** Trials are counted in
> Postgres inside the submission transaction; the research cutoff clips every read
> (a research project sees 41,395 BTC-USD 1m bars where the Desk sees 102,051);
> scoped tokens cannot reach trading or the holdout, enforced by both code and a
> database trigger; and the sandbox holds (read-only root denied, no egress, all
> capabilities dropped).
>
> **The blocker for the agent half:** a live research session needs an Anthropic
> credential in the encrypted store (`CRED_KEK` plus a stored key). This session did
> not have one and did not create one. Everything either side of that gap is
> verified — SDK options build against the real pinned SDK, the image runs, `tbot`
> works — but no agent has yet run a session.
>
> Remaining, in the order I would take it:
> 1. Add a provider credential, then run the end-to-end spike (AGENT-001 §23.4) —
>    container + proxy + `tbot data bars` + `tbot jobs` + a validated report.
> 2. The **pure-noise eval task** (AGENT-004). It is the sharpest honesty test in the
>    design and cannot be faked by prompting.
> 3. The orchestrator's HTTP surface and the event bridge, then the workspace UI.
> 4. The rest of DATA-005 §5, then Sets M and Q.

1. ~~**Ask the user to review and accept ADRs 0024–0030.**~~ **Done** — all Accepted.
2. **Ask the user to confirm the open questions** in 00_INDEX "Cross-cutting open
   questions", especially:
   - **Desk project design** (DATA-005 §4, ADR-0025): introduced during the cleanup and
     not yet confirmed by the user;
   - billing path for model calls (API key via proxy is assumed);
   - GPU availability;
   - monthly eval budget.
3. **Runtime spike** (17 §1; AGENT-001 §23). Verify every identifier marked *(verify)*
   in AGENT-001 against the pinned `claude-agent-sdk` version:
   - base-URL and auth-token env vars through the proxy;
   - session resume;
   - streaming input for steering;
   - PostCompact hook;
   - cache TTL options;
   - sub-agent caps;
   - the role header.

   Build it throwaway: container + minimal proxy + `tbot data bars` with the cutoff +
   `tbot jobs submit/wait` + a validated report + the pure-noise eval task. Update
   AGENT-001 with the findings.
4. **Ops: deep history backfill** (DATA-005 §11, S1). Target ≥ 3 years of 1m for BTC-USD
   and ETH-USD from free sources. Every later phase needs it.
5. **Verify the suspected `market_bars` data-loss bug** (DATA-005 §3). A task chip was
   offered to the user in the Claude app. **Don't run `OPTIMIZE` or drop anything on the
   production table without the user's approval.** The bug:
   - `clickhouse/02_bars.sql` is `ReplacingMergeTree(revision) ORDER BY (instrument_id,
     available_time)` with no `timeframe`, yet several timeframes are stored in the table
     (`crates/backtest/src/store.rs` filters by timeframe);
   - so 1m and 1h bars closing at the same instant may collapse on merge;
   - this may explain the thin 1h history (G-13).
6. **Reconcile Set K** (`docs/plans/plan-sets/set-K/MASTER.md`): part shipped with
   FEAT-003 Phase 1. Fold the remaining items into Sets L and M.
7. ~~**Write plan set L**~~ **MASTER and Phase 0 are written**; phase files 1–6 remain.
   Follow the Set J format (`docs/plans/plan-sets/set-J/`). The scope is in 17 §2:
   - AGENT-001, AGENT-002 (core), AGENT-004 (skeleton);
   - COMP-005, COMP-006 (v1);
   - the DATA-005 core, and the AGENT-003 core skills and findings.

   Every task cites requirement IDs. Then add the set to `docs/plans/README.md`.
8. Later sets: M (engine truthfulness) and Q (research standard) in parallel, then N, O,
   R-a, P, R-b, plus the Track for live evaluation (G-11). The dependency graph is in
   17 §3.

## 6. Gotchas

- **Uncommitted user changes, do not touch or commit:** `Cargo.lock`, `Cargo.toml`,
  `apps/platform/src/main.rs`, `crates/config/Cargo.toml`, `crates/config/src/lib.rs`.
  They predate this work.
- **Case-insensitive filesystem vs git:**
  - git tracks `docs/specs/` in **lowercase**; Windows shows it as `docs/Specs`. Always
    write links as `docs/specs/…`, or they break on GitHub.
  - **`docs/specs/README.md` and `docs/specs/README.MD` are both tracked** and are one
    file on Windows. Keep them identical when committing:
    `h=$(git hash-object -w docs/specs/README.md); git update-index --cacheinfo 100644,$h,docs/specs/README.md; git update-index --cacheinfo 100644,$h,docs/specs/README.MD`.
- **Docker Desktop wasn't running** on 2026-09-11. See the phantom-socket crash-loop
  workaround in the project memory notes if it fails to start. The platform runs on port
  7080.
- **Cargo on this box:** build and test with `-j 2`; full parallelism exhausts the page
  file. Watch disk space (`target/` grew to 61 GB before). Never delete the Docker VHDX.
- **Numbers in the docs:** token and dollar figures (04, R4) are estimates, not
  measurements. Paper and arXiv results are as reported by their authors.
- **Free sources (ADR-0027):** confirm each provider's current terms, rate limits and
  US access before building an adapter. Binance returned HTTP 451 earlier.
- **Nothing in any spec gives the agent trading, automation, alias-promotion or holdout
  access.** Keep it that way (D-12, ADR-0025).

## 7. Conventions used

- Requirement IDs are stable. Cite them in plans and PRs (e.g. `RT-17`, `JB-02`).
- Enforcement tags: **(E)** platform service, **(H)** harness hook, **(A)** agent,
  checked by the validator or evals.
- Set J invariants INV-1/2/3 and ADR-0023 P1 hold everywhere.
- Commits end with `Co-Authored-By:` for the model used. The user asks before commits and
  pushes; branch off `main` for new work.
