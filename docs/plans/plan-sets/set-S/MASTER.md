# The Agent Platform — Set S

**Completion: 14% left (140 / 189 tasks). 140 done, 48 partial, 0 not started, 0 blocked upstream, 1 blocked on a human.** Re-counted 2026-10-05 (track VERIFY2) from the per-task markers; the line read 137 / 189 before.

> This line read *"this header was wrong and is superseded by the per-phase
> files"* until 2026-09-21. Deferring to the phase files was the right instinct
> and the wrong remedy: a MASTER with no number is a MASTER nobody can check,
> and the two sibling sets spent six days claiming `0% (0 / N tasks)` while
> their own phase files recorded real work. The number here is now **derived**
> from those phase files by `every_set_master_counts_its_own_set` and fails the
> build when it drifts — which is the property the sentence was reaching for.

> **Every phase has now been audited against the source (2026-09-16).** Read the per-phase
> header, not this line. The audit ran in two passes: phases 4, 5 and 6 read task by task, and
> then every phase's `**Files:**` list checked for existence with brace and glob expansion.
>
> What it found, and it went one way only: **phases 4, 5 and 6 were recorded as `NOT STARTED`
> and were substantially built.** Phase 4 has roughly 6,800 lines of memory and eval code over
> 98 passing tests; phase 5's `crates/features` is a real engine of about 5,100 lines over 101
> tests that the simulator already resolves through; phase 6's desk is 18 components and 13
> routes over roughly 5,300 lines. Going to implement a task and finding it already built, with
> its adversarial test passing, is how this was noticed.
>
> Phases 0, 1, 2 and 7 were already tracked honestly and needed no correction — phase 7's note
> that the monitor "decides correctly but does not yet measure" is the standard the others are
> now held to. Phases 8, 9, 10 and 11 really are at 0%: the file audit confirms
> `crates/research-session/`, `crates/harness/src/report/`, `crates/harness/src/lifecycle/` and
> most of `crates/harness/src/context/` do not exist.
>
> **The failure mode this file had is worth naming, because it is the expensive one.** A task
> recorded as not started when it is built costs a reimplementation and, worse, hides the real
> bottleneck: phase 5's was never the missing engine, it was one sentence in the MCP server's
> `initialize` instructions telling every model that "backtests support only ema_N and rsi_N
> features" while fifty families resolved. A plan that is wrong in the "not started" direction
> sends people to build what exists and leaves what is actually blocking untouched.
>
> **A second pattern, found the same day and worth more than the first.** Four of
> the five defects that mattered were not missing code but **unreached** code:
> `assemble_brief` with 936 lines, 14 passing tests and no caller anywhere;
> `crates/features` resolving fifty families while the server's own instructions
> said two; thirteen `/api/…` endpoints the desk calls that nothing serves; a
> `memory_recall` field and a `Retrieval` type built around a table that did not
> exist. Each looked exactly like working code, because its own tests passed.
>
> A green suite says every module is correct. It says nothing about whether any of
> them are connected. Before building anything downstream of a module, check it
> has a caller — and prefer a test to an audit, because a list maintained by
> looking is wrong by the amount nobody looked. The frontend/backend route diff
> gave three answers on three passes (3, 12, 13) before it became
> `crates/api/tests/the_desk_calls_routes_that_exist.rs` and stopped needing a
> human to run it.

**Status:** DRAFT (2026-09-14) — ready for execution.
**Created:** 2026-09-14
**Builds on:** Set L (`docs/plans/plan-sets/set-L/MASTER.md`) — Set S rebuilds the agent
runtime Set L delivered, on the evidence of what running it revealed.
**Derived from:** the 2026-09-14 audit recorded in
[AGENT-005 §0](../../../specs/AGENT-005-context-assembly.md),
[AGENT-006 §0](../../../specs/AGENT-006-tool-catalog.md),
[AGENT-010 §0](../../../specs/AGENT-010-agent-runtime.md), and the research brief
[quant-agent-toolbox-uncommon-tools.md](../../../research/quant-agent-toolbox-uncommon-tools.md).
**Formal specs:** [AGENT-005](../../../specs/AGENT-005-context-assembly.md),
[AGENT-006](../../../specs/AGENT-006-tool-catalog.md),
[AGENT-007](../../../specs/AGENT-007-research-memory.md),
[AGENT-008](../../../specs/AGENT-008-reasoning-protocol.md),
[AGENT-009](../../../specs/AGENT-009-evaluation-and-calibration.md),
[AGENT-010](../../../specs/AGENT-010-agent-runtime.md),
[AGENT-011](../../../specs/AGENT-011-strategy-authoring-for-models.md),
[AGENT-012](../../../specs/AGENT-012-research-desk-ui.md),
[AGENT-013](../../../specs/AGENT-013-modes-and-research-lifecycle.md),
[AGENT-014](../../../specs/AGENT-014-continuous-research.md),
[COMP-007](../../../specs/COMP-007-live-monitor-and-kill-switch.md)
**ADRs:** 0034, 0035, 0036, 0037 (Proposed 2026-09-14); ADR-P7-01…05 (reconstructed, in
`docs/ml-platform-context-pack/decisions/ADR-P7-web-skills-evidence.md`)
**Scope class:** End-state architecture. Every subsystem is specified at full fidelity;
phases are build-ordering, not feature-gating (the Set J convention).

---

## 1. Overview

Set L put an agent on the platform. Running it produced measurements, and the
measurements say the agent is capable in the wrong places: it re-reads 51 KB of
identical prefix on every hosted call while a 7B model loses a quarter of its working
state to tokens nobody counted; it can refute a strategy but cannot compute a Sharpe
standard error; it forgets everything it established the moment a turn ends; and the
only thing it can actually *say* in a strategy is `ema_N` and `rsi_N`.

Set S fixes the layer under all of that. Its thesis: **the agent's honesty, its economy
and its capability are all properties of the harness, not of the model.** A 7B and a
frontier model should run the same loop, through the same catalogue, against the same
budget arithmetic, and produce evidence in the same shape — the model decides *what to
do*, and the platform decides what is affordable, what is exposed, what is verified and
what is remembered.

**What this set delivers:**

- **Phase 0 — Ground rules.** The tree is committed, one live approval defect is fixed,
  and the nine specs get the `§6.1.x … Verified by:` acceptance form the execute-plan
  evidence chain requires. **(GR-01…GR-06)**
- **Phase 1 — Contracts.** One tool declaration (`ToolSpec`/`ToolResult`), a charter cut
  from 15.5 KB to 3.2 KB, prompt caching, budget arithmetic that counts the schemas, and
  a harness-owned `WorkingState` the model reads instead of reconstructing.
  **(TC-01…TC-08, CA-01…CA-08, RP-01…RP-04)**
- **Phase 2 — One runtime.** The frontier tier moves onto the harness loop behind a
  flag, the effect log becomes the run's durable state, and `driver.rs` is deleted.
  **(AR-01…AR-12)**
- **Phase 3 — The catalogue.** The templated tool tier, the whole `analysis` namespace,
  the honesty arithmetic, the leakage detectors, the adversarial battery and execution
  realism. **(TC-11…TC-37)**
- **Phase 4 — Memory and evaluation.** Findings that survive a turn, negative results
  that stop the agent re-testing dead ideas, and replayable trajectories that make every
  profile claim measured rather than declared. **(RM-01…RM-12, EC-01…EC-11)**
- **Phase 5 — The language.** One feature engine, a text strategy language a 7B can
  write under a grammar, and `preview_signals` so a rule is seen behaving before a trial
  is spent. **(SA-01…SA-16)**
- **Phase 6 — The Research Desk.** One surface replacing three, built on the Meridian
  primitives that already exist. **(DK-01…DK-22)**
- **Phase 7 — The monitor.** An independent service that watches the book and can stop
  it, which the agent may query and never skip. **(MS-01…MS-11)**
- **Phase 9 — Context as retrieval.** Assembly stops being a chronological buffer that gets
  trimmed and becomes retrieval over a run store: typed records, semantic degradation instead
  of byte truncation, an always-present manifest so a drop never hides that something exists,
  `expand`/`focus`, budget that follows step value, and a versioned policy fitted to measured
  utility and A/B tested offline over real trajectories. **(CX2-01…CX2-12)**
- **Phase 8 — Integration and honesty.** The composed workflows, the gate amendments
  that make a gate refuse without evidence, the report layer, and `run_python`.
  **(TC-35…TC-40, RP-05…RP-08, EC-12)**
- **Phase 10 — Modes and the lifecycle.** Two user-facing modes (Chat, Research — with a Continuous toggle)
  and the ten-stage lifecycle every trial-spending run passes through: pre-registration sealed
  before the first trial, hypotheses that name their falsifier, verdicts the platform computes,
  refinements that answer a diagnosis, proposals as the only door into rings 1 and 2, and a
  yield per run so "quality of research decisions" is a scorecard row. **(LC-01…LC-16)**
- **Phase 11 — Continuous research.** Research with the toggle on: a charter, a scored agenda
  fed by memory and market observations, budget leases, schedule windows, a stall detector, a
  digest, and `replenish` — which is what "endless" means. Frontier-only. **(PG-01…PG-14)**

**Why Phase 2 is early and is not split.** AGENT-005 §5.2 as written tells an
implementer to build session-scoped exposure into `crates/api/src/agent/driver.rs`, and
AGENT-010 §2 then deletes that file. Executed in spec order, one team writes several
hundred lines that another deletes. **This set overrides both spec build orders on that
point:** the frontier tier moves onto the harness loop *before* any routing work is done
for it, so AGENT-005 §5.2/§5.3/§6.3 are satisfied once, in `crates/harness`, by code that
survives. This ordering constraint does not appear in either spec and is recorded here.

---

## 2. Scope

### In scope

| Area | Today | Set S delivers |
|---|---|---|
| Runtime | three loops: `driver.rs` (hosted), `harness::drive::Loop` (local), the SDK container (Claude-only, image unbuilt) | one loop, every tier; the container demoted to the `run_python` sandbox |
| Context | 51 KB byte-identical prefix per hosted call, uncached, unrouted; charter is 28% of a 7B's budget and uncounted | one budget that counts every byte; stable-first layers with cache breakpoints; routed exposure at every tier |
| Tool declaration | four hand-maintained arrays plus a fifth for approvals; `promote_model_version` ungated | one `ToolSpec`; taxonomy, profiles and envelopes generated and tested against it |
| Catalogue | 66 tools, no statistics suite, no leakage detectors, no execution realism | ~60 additions across typed, templated and workflow tiers |
| Memory | a prose "you concluded" string the hosted driver ignores | episodic turn summaries, verified findings, derived negative results, a retrieved task brief |
| Reasoning | events in a transcript; plans may name non-tools | `WorkingState`; plan → act → verify → record; tool-shaped plans; decisions remembered |
| Evaluation | one eval needing a live Ollama, which once scored a healthy tier 0/10 through its own bug | replayable trajectories, per-tier scorecards, a non-inferiority gate, calibrated profile claims |
| Strategy surface | `ema_N` and `rsi_N`, a comparison grammar with no functions | DATA-006 F1 engine, SLv2 text, `preview_signals` before any trial |
| Human surface | `/agent` (polling) and `/research` (SSE), both on deprecated Tailwind aliases | one Research Desk on the Meridian primitives |
| Live safety | an in-process `AtomicBool` nothing trips | an independent monitor with input-first limits and a fail-closed lease |
| Modes and lifecycle | a composer control mixing a goal kind, a workflow and an editor; a run that can choose its evaluation after the result | Chat / Research with a Continuous toggle; pre-registration with a falsifier before any trial; platform-computed verdicts; refinement chains that stop; four enforced rings; yield per run |
| Continuous research | nothing chooses the next question; `mlops.campaign` optimises one family | `ResearchSession { continuous }`: a charter, a scored agenda with a forced exploration share, leases, windows, a stall detector, a digest, `replenish` |

### Out of scope (this set)

- **Models and prediction series** — Set N (`MD-`). Unchanged by Set S.
- **New data sources and the deep backfill** — Set P (`DA-10…DA-12`, S1–S7). Unchanged.
- **Multi-instrument / cross-sectional strategies** (OQ-040) — after SLv2 v2.0
  (AGENT-011 §9).
- **Trading, automation arming, holdout access for the agent.** Never in scope, in any
  set (D-12, ADR-0025). Set S *strengthens* this: `spend_holdout_look` is the only door
  into holdout data and carries a `SealedHoldout` envelope.
- **LLM-derived text features** and `llm_contamination_check` — specified in the research
  brief, deferred until a text feature exists (AGENT-006 §5.10).

### Reserved set letters this set absorbs

`17_BUILD_ORDER.MD §2` reserves M, N, O, P, Q, R-a, R-b. Set S is deliberately a
different cut — it is organised around the agent rather than around subsystems — and it
absorbs parts of four reserved sets. Recorded so the roadmap stays honest:

| Reserved | Was | Set S takes | Remains |
|---|---|---|---|
| **M** Engine truthfulness | `FE-`, `BT-`, `ST-` Layer 1 | the DATA-006 F1/F2 feature engine and the one-engine rule (Phase 5) | backtest-engine truthfulness beyond features |
| **O** Strategy Language v2 | `ST-05…ST-12` | SLv2 as the model-facing language, compiler, moves (Phase 5) | the visual builder's v2 surface |
| **Q** Research standard | `EV-01`, `EV-03…EV-08` | the honesty arithmetic and the leakage battery (Phase 3, 8) | the remaining EV- extensions |
| **R-a / R-b** Skills, knowledge | `SK-`, `KM-` | episodic + semantic memory, playbook admission (Phase 4); campaigns as the `optimise` workflow over the built `mlops.campaign` and autonomy as continuous research (Phases 10, 11) | the regime engine |
| **N**, **P** | — | nothing | whole |

---

## 3. Locked decisions carried into this set

| ID | Decision | Source |
|---|---|---|
| D-S1 | One loop for every tier; `driver.rs` is deleted, not maintained | ADR-0035 |
| D-S2 | The container is the code sandbox, not the agent's runtime | ADR-0035, reverses ADR-0024 §1 |
| D-S3 | Every byte sent to a model is charged against the profile budget before the body is sized | AGENT-005 P1 |
| D-S4 | The catalogue may be any size; the prompt may not. Growth is absorbed by routing | AGENT-006 P1 |
| D-S5 | Memory is written by tools from verified results, never by the model's prose | AGENT-007 P1 |
| D-S6 | The checking mechanism is independent of the thing it checks — separate process for the kill switch, separate session for the critic, a real tool call for claim verification | ADR-0034 §2, COMP-007 P1 |
| D-S7 | A tool that runs the simulator registers its trial before it runs | INV-16, AGENT-006 P5 |
| D-S8 | Profile claims are measured. An unmeasured tier reports itself `unmeasured` | AGENT-009 P3 |
| D-S9 | Rigor lives in the tool, so a 7B and a frontier model get the same statistics | AGENT-006 P8, AGENT-008 P1 |
| D-S10 | No agent token may trip or reset the kill switch, or read holdout data outside `spend_holdout_look` | COMP-007 §5, AGENT-006 §2.6 |
| D-S11 | Two modes — Chat, Research — with a Continuous toggle on Research; "endless" is the toggle's `replenish` setting; continuous research is frontier-only and defaults to `notify` | ADR-0037 |
| D-S12 | Every trial-spending run is pre-registered with a falsifier before its first trial; the platform, never the model, writes the verdict; rings 1 and 2 are reached only by a proposal a human decides; ring 3 has no door | AGENT-013 P2, P3, P6 |

---

## 4. Phase summary

| Phase | Title | Tasks | Requirement IDs | Parallel with | State |
|---|---|---|---|---|---|
| 0 | [Ground rules](phase-0-ground-rules.md) | 6 | GR-01…GR-06 | — (blocks all) | **1 / 6 done · 3 partial** |
| 1 | [Contracts](phase-1-contracts.md) | 20 | TC-01…TC-10, CA-01…CA-09, RP-01…RP-07 | three internal tracks | **17 / 20 done · 3 partial** |
| 2 | [One runtime](phase-2-one-runtime.md) | 16 | AR-01…AR-16 | 6, 7 | **5 / 16 done · 9 partial** |
| 3 | [The catalogue](phase-3-catalogue.md) | 19 | TC-11…TC-37 | 4, 5, 6, 7 | **7 / 19 done · 7 partial** |
| 4 | [Memory and evaluation](phase-4-memory-and-evaluation.md) | 17 | RM-01…RM-15, EC-01…EC-13 | 3, 5, 6, 7 | **7 / 17 done · 9 partial** |
| 5 | [The language](phase-5-strategy-language.md) | 15 | SA-01…SA-16 | 3, 4, 6, 7 | **2 / 15 done · 3 partial** |
| 6 | [The Research Desk](phase-6-research-desk.md) | 24 | DK-01…DK-24 | 2, 3, 4, 5, 7 | **8 / 24 done · 16 partial** |
| 7 | [The monitor](phase-7-monitor.md) | 13 | MS-01…MS-14 | everything | **5 / 13 done · 7 partial** |
| 8 | [Integration and honesty](phase-8-integration.md) | 9 | TC-35…TC-40, RP-05…RP-08, EC-12 | — (needs 2,3,4) | **7 / 9 done · 2 partial** |
| 9 | [Context as retrieval](phase-9-context-as-retrieval.md) | 20 | CX2-01…CX2-20 | 8 | **17 / 20 done · 3 partial** |
| 10 | [Modes and the lifecycle](phase-10-lifecycle.md) | 16 | LC-01…LC-16 | 9 | **6 / 16 done · 10 partial** |
| 11 | [Continuous research](phase-11-continuous-research.md) | 14 | PG-01…PG-14 | 9 | **0 / 14 done · 3 partial** |

> **Every row re-derived 2026-09-21 from the phase files' own task markers.** This
> column used to be stale for every row but 8 — phases 4, 5, 6, 9 and 10 read
> `NOT STARTED` while substantially built, found on 2026-09-16 by going to implement
> a task and discovering its adversarial test already passing.
>
> It is a hand-copied table, so it will drift again. What stops it mattering is that
> the counts are now held by
> `crates/invariants/tests/the_plan_counts_what_the_plans_say.rs`: each phase file's
> own headline must match its own tasks, and `docs/plans/PLAN.md` must match the sum.
> If this table disagrees with those, they are right and this is stale — check there
> first, and prefer [PLAN.md](../../PLAN.md) over any hand-written count anywhere.

### Dependency graph

```
Phase 0 ──▶ Phase 1 ──▶ Phase 2 ─────────────────▶ Phase 8
              │            │                         ▲
              ├──▶ Phase 3 ┼─────────────────────────┤
              ├──▶ Phase 4 ┘                         │
              ├──▶ Phase 5 ────────────────────────▶ ┘
              └──▶ Phase 6 (starts on today's data, deepens as 2–5, 7 land)
Phase 7 ──────── independent from Phase 0; only its UI page waits on Phase 6
Phase 9 ──────── needs 1, 2 and 4; runs beside 8; must beat Part I to ship
Phase 10 ─────── needs 1, 2, 4 and 8 (its desk task needs 6); runs beside 9
Phase 11 ─────── needs 10, 2, 4 and 7 (its desk task needs 6); runs beside 9
```

**Five tracks can run concurrently after Phase 1:** runtime (2), catalogue (3),
memory+evals (4), language (5), desk (6), monitor (7). Phase 7 can start at Phase 0.

---

## 5. The agent contract

Every implementing agent reads, in this order: `docs/procedures/AGENT.md`, this file, its
phase file, then the spec sections its tasks trace to. Then:

**Workspace discipline**
- One track per git worktree, one branch per phase, named `claude/set-s-phase-<n>-<topic>`
  (CI runs on `claude/**` and on PRs to `main`).
- Never start a task without setting it `IN PROGRESS` in the phase file; never start the
  next one while the current is `IN PROGRESS` (`docs/procedures/execute-plan.md`).
- Reserved migration ranges — **take numbers only from your own range** so concurrent
  tracks cannot collide (latest existing was `0060_web_and_skills.sql` when this was
  written).

  > **The table below is a record of what was reserved, not of what was used.** The
  > tracks did not run concurrently and the repository is past `0130`; phase 8 took
  > `0134`–`0136`, phase 10 took `0127`–`0133`. Reserving a range only prevents a
  > collision between tracks running at the same time, and once one track is ahead the
  > next number is the next number. Kept rather than rewritten, because a reader
  > finding `0085` cited in a phase file needs to be able to see where that came
  > from.

  | Track | Range |
  |---|---|
  | Phase 2 runtime | 0061–0064 |
  | Phase 4 memory | 0065–0069 |
  | Phase 4 evaluation | 0070–0073 |
  | Phase 7 monitor | 0074–0078 |
  | Phase 3 catalogue | 0079–0081 |
  | Phase 5 language | 0082–0084 |
  | Phase 8 integration | 0085–0088 (used `0134`–`0136`) |
  | Phase 10 lifecycle | 0089–0091 |
  | Phase 11 continuous research | 0092–0096 (used `0138`) |

- Shared files that will conflict across tracks — coordinate, do not rebase blindly:
  `Cargo.toml` (workspace members), `Cargo.lock`, `crates/mcp-server/src/taxonomy.rs`
  (generated from Phase 1 onward), `crates/api/src/routes/mod.rs`,
  `frontend/src/App.tsx`, `frontend/src/components/layout/TopBar.tsx`.

**Verification — a task is done when every one of these is true**
1. The behaviour matches the spec section the task traces to.
2. The spec's acceptance criterion has its `Verified by:` filled with the test name
   (`docs/procedures/execute-plan.md` step 3c).
3. Every *decided mechanism* has an adversarial test proving the violation is **rejected**,
   not only that the happy path works (`AGENT.md` invariant 8;
   `ml-platform-context-pack/CLAUDE.md` §5.3).
4. No invariant regressed.
5. A judgment call, if any, is recorded in `decisions/ADR-INDEX.md`.

**Commands.** `cargo install just sqlx-cli` once, then use the recipes — they are what
CI runs. `.cargo/config.toml` already pins `jobs = 2`; **never raise it** (see hazards).
The toolchain is pinned to 1.96.0 by `rust-toolchain.toml`.

```bash
just fmt-check          # cargo fmt --all -- --check
just lint               # cargo clippy --workspace --all-targets -- -D warnings
just test-unit          # cargo test --workspace --lib   <- the safe one, no infra
just check-money        # cargo xtask check-money-f64
```

Then the CI jobs your track can break. All eleven run on every PR to `main` and on
`claude/**` pushes; there is no `needs` chain, so they all run:

```bash
cargo test -p invariants --test static_invariants     # scans EVERY .rs under crates/ and apps/
cargo test -p ledger -p dataplane
cargo test -p harness --test conformance              # a profile field nothing reads fails here
cargo test -p harness --test failure_injection
cargo test -p mcp-server-lib --lib taxonomy           # Phase 1 rewrites what this guards
cargo test -p api --test auditor_suite
cargo test -p features --lib leakage
cargo xtask check-bars-v1-frozen
cargo tree -p builders --edges normal                 # builders-purity: no I/O deps in builders/features
python -m pytest evals/tests -q                       # an eval harness already exists, in Python
```

Infra-backed suites (`just infra` then `just migrate`; CI supplies
`pgvector/pgvector:pg16` + ClickHouse):

```bash
cargo test -p ledger -- --ignored --test-threads=1
cargo test -p invariants --test db_invariants -- --ignored --test-threads=1
```

Frontend (Phase 6): `cd frontend && npm run lint && npm run build && npm test`. Note
`npm run build` runs `tsc -b`, so it is the typecheck; it is the only hard gate in
`frontend.yml` (lint and typecheck are `continue-on-error`).

**Hazards, measured on this machine — read before your first build**
- **`target/` is ~237 GB.** Check free space before any full rebuild. `cargo clean` costs
  a rebuild of 639 crates, four of them fetched from the pinned `market_simulator` git
  SDK over the system git binary. Never delete a Docker VHDX to make room.
- **Raising `-j` reproducibly OOMs** the Windows page file and surfaces as misleading
  `E0463 can't find crate` / `E0786 invalid metadata` cascades that look like source
  errors. Leave `jobs = 2`. Never pipe cargo through another command — it hides the exit
  code.
- **`xtask` scans `git ls-files`**, so `check-money-f64` and `check-bars-v1-frozen` skip
  files you have not `git add`ed. A new file passes locally and fails CI once committed.
  `git add` before you trust a green run.
- **Migration checksums are byte-exact and CRLF breaks them.** `.gitattributes` pins
  `migrations/*.sql text eol=lf` after this bit migration 0036 on 2026-09-14. If an
  existing database rejects a migration, repair with
  `cargo run -p storage --example normalize_checksums` (it refuses anything that is not
  provably a line-ending difference). Migrations are `include_str!`-embedded at compile
  time, so editing a `.sql` needs a rebuild of `storage`.
- **`docs/specs/` is tracked lowercase** even though Windows shows `docs/Specs`. Always
  write links lowercase, or a case-sensitive checkout gets two directories.
- **The platform needs four things to boot** that have no defaults:
  `PLATFORM_DB_APP_PASSWORD`, `PLATFORM_LEDGER_SIGNING_KEY` (64 hex,
  `openssl rand -hex 32`), `PLATFORM_WORM_DIR`, and a reachable ClickHouse. It serves on
  **7080** (`config/local.toml` overrides the 8080 in `default.toml`; the Vite proxy
  targets 7080). It refuses to bind a non-loopback address by design.
- **ClickHouse reports false-unhealthy** on boot (the alpine image has no `wget`). Wait;
  do not restart it. Docker itself crash-loops on un-deletable phantom sockets — rename
  `Docker
un` aside before launching.
- **Pre-existing failures. Do not chase them, do not "fix" them into your PR:** four
  `storage` live suites (`ledger_append`, `ledger_writer`, `pnl_schema` want a
  `ledger_events` table no migration creates; `registry_seed` expects 8 asset classes
  where 0006 seeds 6) and `cargo xtask lint-no-json-hotpath`. All were failing before
  Set S. Record them, leave them.
- **The working tree is dirty and some of it is the user's.** `BS-007/HANDOFF.MD` §6
  names files an agent must not commit: `Cargo.lock`, `Cargo.toml`,
  `apps/platform/src/main.rs`, `crates/config/Cargo.toml`, `crates/config/src/lib.rs`.
  S-0.1 sorts this out with the user before any track branches.

**When a spec is wrong.** It will be. Do not silently deviate and do not work around it.
Mark the task `BLOCKED:UPSTREAM`, write the correction into the spec with a dated note,
set every plan citing that spec to `Derivation Status: STALE`, re-derive the affected
tasks, then resume (`docs/procedures/add-plan.md`, re-derivation section). A deliberate
deviation that is *not* a spec error is declared in the task, in the Set L form:
"**S-2.4 done — as a dual-write, which is a deliberate deviation from this task as
written**", with the reason.

**Progress.** Update the phase file's completion line and the `§7 Progress log` here at
every milestone boundary. Report partial as partial; a partially-enforced invariant is
never reported as enforced.

---

## 6. Exit criteria

Set S is complete when all eleven specs' acceptance sections pass with evidence:

1. **AGENT-005 §10** (10 criteria) — the charter is ≤ 3,200 B; a 30-iteration hosted run
   reads ≥ 80% of its prompt from cache; exposure never exceeds the profile budget; a 7B's
   body budget holds its 25% floor with six schemas reserved.
2. **AGENT-006 §10** (10) — one `ToolSpec` per tool with generated taxonomy/profile/envelope
   arrays; every description within budget; no namespace over 16; every simulator-running
   tool spends a trial.
3. **AGENT-007 §8** (8) — a second conversation receives the first's established findings
   with ids, on a 7B and on the frontier tier; a negative result cannot carry a class the
   funnel did not produce.
4. **AGENT-008 §7** (7) — working state is rewritten not appended; a non-tool plan step is
   rejected; the LOCAL_TIER_FINDINGS §8 truncation fence cannot recur.
5. **AGENT-009 §8** (7) — every live trajectory replays to identical effects; the 0/10 eval
   bug exists as a passing replay case; a PR regressing the 7B is blocked even if the
   frontier improves.
6. **AGENT-010 §14** (9) — one driver; a killed process resumes with no duplicated write
   effect; a run waiting on a 40-minute sweep spends zero tokens.
7. **AGENT-011 §8** (8) — one feature engine serves backtest, runtime and templates; SLv2
   round-trips; a 7B compiles a strategy in ≤ 6 fills.
8. **AGENT-012 §19** (8) — a running turn is understandable from the state panel alone;
   every id resolves or renders struck-through; the replay scrubber reproduces the prompt
   byte-for-byte.
9. **COMP-007 §8** (8) — killing the monitor stops order flow within one cadence; no agent
   token can call a monitor POST.
10. **AGENT-013 §13** (15) — no trial without a sealed pre-registration and none from a Chat
    turn; a hypothesis without an evaluable falsifier is refused; the verdict is
    platform-written; the only door into rings 1 and 2 is a human-decided proposal; every run
    has a yield.
11. **AGENT-014 §12** (12) — a killed session driver resumes with one child and an identical
    fold; the exploration share cannot go below 0.10; a tripped kill switch pauses every
    continuous session; a stalled session says why and waits; a bounded Research run is the
    same object with one agenda item.

**Gate on every phase exit:** the eval suite (Phase 4) scorecard shows no tier regressed
(`17_BUILD_ORDER §4`: "the eval suite gates every phase exit"). Phases 0–3 run before that
gate exists and are re-checked retroactively at Phase 4 exit.

---

## 7. Coverage

Forward check (every acceptance criterion has a task) and backward check (every task
traces to a source) per `docs/procedures/add-plan.md` step 5.

| Spec | Criteria | Tasks covering them |
|---|---|---|
| AGENT-005 Part II | §30.1–30.20 | S-9.1…S-9.20 |
| AGENT-005 | §10.1–10.10 | S-1.5…S-1.12, S-2.2, S-2.4, S-2.5, S-2.7, S-2.13, S-4.12 |
| AGENT-006 | §10.1–10.11 | S-0.2, S-1.1…S-1.4, S-3.1…S-3.14, S-8.1…S-8.3, S-11.14 |
| AGENT-007 | §8.1–8.9 | S-4.1…S-4.7, S-4.15, S-10.12 |
| AGENT-008 | §7.1–7.8 | S-1.13…S-1.17, S-2.12, S-2.14, S-8.5…S-8.7, S-10.1, S-10.2, S-10.14 |
| AGENT-009 | §8.1–8.8 | S-4.8…S-4.14, S-8.8, S-10.16 |
| AGENT-010 | §14.1–14.10 | S-2.1…S-2.14, S-10.11 |
| AGENT-011 | §8.1–8.8, §8.5a | S-5.1…S-5.14 |
| AGENT-012 | §19.1–19.10, §19.1a | S-6.1…S-6.23, S-10.15, S-11.13 |
| COMP-007 | §8.1–8.10 | S-7.1…S-7.12 |
| AGENT-013 | §13.1–13.15 | S-10.1…S-10.16 |
| AGENT-014 | §12.1–12.12 | S-11.1…S-11.14 |

Gaps recorded rather than hidden: AGENT-005 §10.2 (cache hit ≥ 80%) and AGENT-009's
`decode` suite require a **live model with a credential**, which Set L never obtained
("the one thing that has never happened: a live agent session"). Task S-4.14 provisions
that credential and runs the first live measurement; until it does, those two criteria
are `BLOCKED:HUMAN` and are the only ones in the set that cannot be closed by an agent
alone.

---

## 8. Risks

| Risk | Mitigation |
|---|---|
| Deleting `driver.rs` breaks the only working hosted path | Phase 2 lands behind `agent.single_loop`; the old driver stays one release; AGENT-009 shadow replay compares outcomes on real trajectories before the flag flips (S-2.9) |
| Five concurrent tracks collide in shared files and migrations | Reserved migration ranges (§5); named shared-file list; the generated taxonomy removes the biggest hand-edited conflict in Phase 1 |
| The templated tier becomes a second, unaudited way to add tools | Templates carry the same `ToolSpec`, the same risk class and the same trial rule; a build-time test asserts template and catalogue schemas agree (S-3.2) |
| An agent "finishes" a task by making the test match the code | DoD condition 3: every decided mechanism needs a test that proves the *violation* is rejected. The honesty suite (S-4.11) plants an oracle and asserts it is killed |
| Spec churn during execution silently invalidates plans | `Derivation Status: STALE` protocol in §5; `add-spec.md` step 5 fires it automatically |
| The desk is built on the deprecated Tailwind aliases like the surfaces it replaces | S-6.1 forbids `bg-surface-2`/`text-text-*`; the desk is built only on `@/components/primitives` and `app.css`; S-6.22 deletes the aliases when the last consumer goes |
| The DATA-006 engine lands as a *second* EMA/RSI implementation | `static_invariants` AT-15 bans exactly that. Phase 5 extends `crates/features/src/{ema,rsi}.rs` rather than adding a parallel engine; a genuine exception needs an allowlist entry *and* an ADR row (S-5.1) |
| The feature engine pulls I/O into `crates/features` | `builders-purity` fails the build if `sqlx`, `clickhouse`, `redis`, `async-nats`, `event-bus` or `storage` enter `builders` or `features`. The engine stays pure; data arrives as arguments (S-5.2) |
| A second eval harness is invented beside the Python one already in CI | `evals/runner/run.py` and `evals/tests` exist and gate CI. Phase 4 extends them for model-in-the-loop suites and adds Rust replay suites only where determinism needs them (S-4.8) |
| A full rebuild fills the disk mid-track | `target/` is already ~237 GB; check free space first, and prefer `just test-unit` over `just test` |
| Phase 3's ~26 tools overwhelm review | Each namespace is one PR with its own acceptance; the description-budget and namespace-cap tests are mechanical gates that cannot be argued past |
| Pre-registration becomes a ceremony the model games (a vacuous falsifier, an amended protocol) | The falsifier must name a metric in the closed vocabulary and a comparator (S-10.4); amendments are lineage, the trial count carries over, and the third blocks the run (S-10.3); the `judgement` suite plants a null and fails a harness that reports it supported (S-10.16) |
| A continuous session left alone burns budget on motion | Every choice is a decision row with its candidate set; a forced exploration share the charter cannot lower; the stall detector pauses on barren streaks, degeneration, circling and cost drift; continuous is frontier-only and cannot go `autonomous` below the `judgement` threshold (S-11.3, S-11.8, S-11.9) |

---

## 9. Progress log

| Date | Phase | Task | Note |
|---|---|---|---|
| 2026-09-14 | — | — | Set S created. All nine specs written and indexed; ADR-0034, ADR-0035 proposed; ADR-P7-01…05 reconstructed. Nothing implemented. |
| 2026-09-14 | — | — | Design review round 2. Twelve revisions across all nine specs; six new tasks (S-1.17, S-2.13, S-2.14, S-4.15, S-6.23, S-7.12) and six amended. Notable: mid-turn tool eviction forbidden (it invalidated the whole prefix cache); the eval gate now uses intervals rather than a bare 2-point threshold; `preview_signals` is cut off at the sealed holdout; contradiction detection added, without which AGENT-007 P5 was an always-empty field; the provenance fence extended to the human's UI. Also closed a forward-coverage gap in this plan: AGENT-008 §7.7 had no implementing task. |
| 2026-09-14 | — | — | Design review round 3. Thirty candidates evaluated, twenty adopted, ten cut and recorded. Eleven new tasks (S-1.18…S-1.20, S-2.15, S-2.16, S-3.19, S-4.16, S-4.17, S-5.15, S-6.24, S-7.13). Load-bearing: the tool-result cap is now bounded by the *remaining* body budget; the estimator calibrates to p90 rather than the median, which was breaking the spec's own over-estimate guarantee; the eval suite gets a sealed 20% holdout, because a harness tuned until its suites pass is a curve-fitted backtest; kill-switch drills, without which fail-closed is a claim. Cut: prefix versioning, eval cost cap, per-user concurrency, stored-strategy migration, cross-conversation search, undo, limit re-baselining, onboarding empty states, workspace hygiene, read auditing. |
| 2026-09-14 | 9 | — | Design review round 4, on context assembly specifically. ADR-0036 and AGENT-005 Part II: assembly becomes retrieval over a run-scoped store rather than accumulation in a buffer. Ten transformational changes — typed record store, query-based selection against the working state, semantic degradation ladders replacing `truncate_middle`, an always-rendered manifest, `expand`/`focus`, step-value-weighted budget, per-GoalKind composed charter, cross-turn prefix continuity, utility attribution, and a versioned ContextPolicy A/B tested offline. Part I becomes ContextPolicy v1 and is the control; v2 ships only by beating it on every tier. New Phase 9 (12 tasks). |
| 2026-09-14 | 9 | — | Design review round 5, on the retrieval design itself. Twenty-seven refinements added to AGENT-005 Part II (§24–§29) and eight tasks to Phase 9. The load-bearing ones are the failure modes a retrieval design has that a buffer does not: selection collapsing onto one subject without a diversity constraint; relevance eating the prompt cache without hysteresis; losing an id the next call needs without deriving hard requirements from the step's tool schema; and silently picking a winner between two disagreeing records. Plus rendering economy (tables are not JSON, precision follows the error bar, the second observation is a delta), an untrusted-share ceiling on the body, counterfactual ablation over tapes, and sub-run context as a projection of the parent store rather than a second pipeline. |
| 2026-09-14 | 10, 11 | — | Design review round 6, on modes and the research lifecycle. ADR-0037: three modes (Chat, Research, Program); "endless" is Program's `replenish` setting, not a Research setting; Diagnose/Falsify/Replicate/Compare/Survey/Optimise are workflows inside Research. AGENT-013 adds the ten-stage lifecycle with pre-registration sealed before the first trial, hypotheses that name their falsifier, platform-computed verdicts, refinement chains that must answer a diagnosis and stop at diminishing returns, four enforced rings with proposals as the only door into 1 and 2, a reproducibility receipt, `compare_candidates` as SPEC §11.5's one door, the `optimise` workflow driving the built `mlops.campaign` through a worker sub-run (the missing `ResearchRun` child worker), open questions as memory, and a yield per run. AGENT-014 adds Programs: a fold on the job service, nine agenda sources, scored selection with a forced exploration share and a decision row per choice, three-layer dedupe, leases, windows, a stall detector, a digest, autonomy behind the `judgement` threshold. Amendments to AGENT-006/007/008/009/010/012; Phases 10 (16 tasks) and 11 (14 tasks); migration ranges 0089–0096 reserved. |
| 2026-09-15 | 10, 11 | — | User decisions on round 6: (1) default autonomy `notify`; (2) exploration floor 0.10 kept, explained; (3) day budget gets a platform default ($25/day from the desk budget policy) instead of being required; (4) no unattended local tiers — continuous research is frontier-only with a revisit trigger; (5) **two modes, not three**: Chat / Research with a Continuous toggle under Research; the continuous machinery becomes qualifiers on Research and the internal object is `ResearchSession { continuous }` with a bounded run as the one-item case. AGENT-014 rewritten (file renamed `AGENT-014-continuous-research.md`), ADR-0037 revised, AGENT-013 §2 rewritten, AGENT-012 §2b rewritten (no `/desk/programs`; Agenda and Charter tabs in the session's conversation), Phase 11 rewritten; the word "Program" removed from every user-facing surface. |
