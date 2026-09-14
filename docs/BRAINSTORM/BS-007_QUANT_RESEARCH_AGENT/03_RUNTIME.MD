# BS-007 / 03 — Agent runtime: Agent SDK, container, orchestrator, workspace, hooks

**Spec status:** spec-ready draft · **Req prefix:** RT- · **Formal spec:** AGENT-001 ·
**ADR:** ADR-0024 (agent runtime), ADR-0025 (platform-side enforcement) · **Plan set:** L ·
**Decisions:** D-02, D-04, D-07, D-10, D-12 · **Replaces:** the internal agent loop in
`crates/api/src/agent/` (01 §8)

> SDK option and env-var names below come from the token audit (research/R4) and the SDK
> docs as of 2026-09. **Verify each against the pinned SDK version in the Phase 0 spike.**
> Items marked *(verify)* are assumptions.

---

## 1. Scope

**In scope:**
- the agent's execution environment;
- the Rust session orchestrator;
- the workspace layout and bootstrap;
- policy hooks;
- sub-agent roles;
- the research protocol state machine;
- `final_report`;
- steering and `ask_user`;
- durability;
- budgets;
- security boundaries.

**Out of scope, owned elsewhere:**
- what enters the model's context and what it costs (04);
- jobs and artifacts (05);
- data access (06);
- tools (14);
- skills (13);
- UI (15).

## 2. Architecture and the enforcement principle

```
┌──────────────── Project container (one per research project; rootless) ─────────────────┐
│ agent-host (Python): Claude Agent SDK client · event bridge · steering inbox            │
│ Built-ins: bash · read/write/edit · grep/glob · sub-agents · skills · compaction · hooks  │
│ /workspace (git): CLAUDE.md NOTEBOOK.md RESEARCH_PLAN.json PROGRESS.md bootstrap.sh       │
│                   research/ strategies/ models/ datasets/ reports/ skill-drafts/ data/    │
│ .claude/skills/ (read-only, materialised)   .claude/settings.json (read-only hooks)       │
│ Python: polars numpy scipy statsmodels arch sklearn lightgbm xgboost torch duckdb plotly  │
│ tbot SDK + CLI · tbot_features (PyO3 feature engine)                                      │
│ Env: TBOT_TOKEN (research scope) · model calls → platform LLM proxy · no other secrets    │
│ Egress: platform API + LLM proxy + package mirror only                                    │
└───────────────┬──────────────────────────────────────────────────────────────────────────┘
                │ HTTPS: /api/* (SDK/CLI, MCP) · /llm/* (proxy) · /agent/events (bridge)
┌───────────────▼────────────────────────── Platform (Rust) ───────────────────────────────┐
│ Session orchestrator │ LLM proxy (auth, budget, telemetry) │ Data API │ Feature engine    │
│ Job service │ Artifact store │ Backtest service │ Set J evaluation │ Model service         │
│ Research memory │ Skill registry │ final_report validator │ SSE to UI │ Approvals          │
└───────────────┬──────────────────────────────────────────────────────────────────────────┘
   Collectors → NATS → ClickHouse / Postgres            Trainer + inference sidecars
```

**Enforcement principle (D-10).** The agent has bash, so it can compute anything on
anything it can read, and it can ignore any prompt. So every invariant is enforced by a
platform service that its token can't bypass:

| Invariant | Enforced by |
|---|---|
| Holdout untouched | Data API research cutoff (06 §3); only the vault gate reads past it |
| Every trial counted (INV-1) | Evaluation service counts every backtest-kind job, whatever client submitted it |
| Sealed distributions (INV-2) | The API has no best-member endpoint |
| Significance needs null and trial count (INV-3) | Set J gates |
| Pre-registration | Experiment creation endpoint (11 §2) |
| Research authority only | Token scopes (§12) |
| Reports cite evidence | `final_report` validator service (§10) |
| Skills verified before use | Skill registry admission (13 §4) |
| Spend limits | LLM proxy and job service budgets (§13) |

Hooks (§7) add fast feedback and defence in depth. Prompts and skills teach. Neither is
the guarantee.

## 3. Container and sandbox

- **One container per research project**, created from a versioned image
  (`tbot-agent:<semver>`). The workspace is a named volume that survives container
  restarts. Containers stop when idle and resume on the next session.
- **Rootless**, with a read-only root filesystem except `/workspace`, `/tmp` and package
  caches. There are CPU, RAM, disk and PID limits per project tier. There is no Docker
  socket.
- **Egress allowlist:** the platform API, the LLM proxy, and a package mirror (PyPI
  proxy). Everything else is blocked at the network layer, not just by hooks.
- **No secrets in the container.** Model calls go through the platform **LLM proxy**
  (`ANTHROPIC_BASE_URL` → `/llm`, *(verify)*). The proxy:
  - authenticates the session token;
  - injects the real API key from the encrypted credential store (exists since
    2026-09-06);
  - enforces the session and project dollar budget;
  - records per-request usage (04 §9).
- **GPU:** not in the agent container. Training runs as jobs on the trainer worker
  (05, 10).
- **Dev box:** Windows with Docker Desktop and WSL2. Note the Docker phantom-socket
  crash-loop (memory note) in the runbook.

## 4. Session orchestrator (Rust)

A new crate, `crates/agent-orchestrator`, mounted in `apps/platform`:

| Responsibility | Detail |
|---|---|
| Project lifecycle | Create project (workspace volume, cutoff, budget, instrument set, memory scope); archive; clone |
| Session lifecycle | Start or resume the container; run the skill materialiser (13 §5); mint a session token; start `agent-host`; stop and snapshot (git commit plus volume snapshot) |
| Event bridge | `agent-host` streams SDK messages (assistant text, tool use and results, sub-agent events, compaction events, usage) → orchestrator → Postgres (`agent_sessions`, `agent_events`) → SSE to the UI |
| Steering | UI message → orchestrator → `agent-host` inbox → injected at the next turn boundary through the SDK's streaming input |
| `ask_user` and approvals | In-process custom tool in `agent-host`. It posts an approval card, blocks with a timeout, and returns the answer or the default |
| Budgets | Session and project limits in dollars (proxy), compute (job service) and wall clock |
| Recovery | On platform boot, sessions that were running are resumed, not failed (replacing `recover_orphans`) |

`agent-host` is a small Python process in the image. It builds the SDK client with:
- system prompt = a stable core (§5 CLAUDE.md, 04 §3);
- `cwd=/workspace`;
- the project's `.claude/settings.json` hooks;
- MCP servers: the in-process `ask_user` tool only, since platform capabilities are reached
  through the `tbot` CLI in bash (14 §1);
- a model pinned per project version;
- effort `high` or above (04 §7);
- a budget cap.

## 5. Workspace layout

```
/workspace
├── CLAUDE.md            # generated per project version: identity pointer, protocol, workspace map,
│                        #   compaction instructions (04 §5); byte-stable within a version
├── NOTEBOOK.md          # agent-maintained; §1 "Current state" ≤ 2k tokens; §2+ detail/archive
├── RESEARCH_PLAN.json   # hypotheses as items: status untested|running|rejected|supported|vaulted;
│                        #   add/change only, never delete (mirrors the platform hypothesis registry)
├── PROGRESS.md          # append-only log; one line per completed step citing job/experiment ids
├── bootstrap.sh         # prints ≤ 40-line status: reachability, coverage + data_qc, pending jobs, cutoff, budget
├── research/  strategies/  models/  datasets/  reports/   # code and specs (git-tracked)
├── skill-drafts/        # proposals only; never loaded as skills
├── data/                # Parquet from the Data API (git-ignored; manifests tracked)
└── .claude/             # skills/ + settings.json, mounted read-only
```

- **NOTEBOOK.md vs the platform:** the notebook is the agent's working view. The platform
  registries (hypotheses, experiments, findings, jobs) are authoritative. `bootstrap.sh`
  reconciles and prints any drift.
- **Git:** one commit per completed step, with a message citing ids. The UI renders the
  history.

## 6. Session lifecycle

| Step | What happens |
|---|---|
| **Initializer** (first session of a project) | Expand the goal into `RESEARCH_PLAN.json` items with pre-registration fields left to fill; write `bootstrap.sh`; baseline commit |
| **Start** | Materialise skills; SessionStart hook runs `bootstrap.sh` and injects its output; the agent reads NOTEBOOK §1 |
| **Work** | The protocol loop (§8). Slow work goes to jobs (05). Findings go to memory as they occur |
| **Checkpoint** | The agent or a hook emits checkpoint events (02 §5). Optional pause for input |
| **Compaction** | Server-side compaction with project instructions (04 §5). NOTEBOOK §1 is re-injected on the next user turn (there is no `PostCompact` hook; AGENT-001 §12, §23) |
| **Stop** | Stop hook: requires a validated `final_report` id, findings for every tested hypothesis, and a clean commit; at most one skill-proposal nudge (13 §3) |
| **Resume** | Any later session starts at **Start**; pending jobs are picked up from the job service |

## 7. Hooks (`.claude/settings.json`, read-only)

| Hook | Rule |
|---|---|
| SessionStart | Run `bootstrap.sh`; inject its ≤ 40-line status |
| PreToolUse (Write/Edit) | Deny writes to `.claude/**` and to paths outside `/workspace` and `/tmp` |
| PreToolUse (Bash) | Deny known secret paths, raw network tools against non-allowlisted hosts, and `docker`/`sudo`; deny `tbot` subcommands outside the token scope (early feedback only; the platform re-checks) |
| PreToolUse (Read) | Duplicate-read guard: if the file hash is unchanged since the last read in this context, return a pointer instead (04 §6). Cleared on compaction |
| PostToolUse | Output guard: results over 8k tokens are spilled to `/tmp/out/<id>.txt` and replaced by a summary and path (04 §6). Log tool usage to the event bridge |
| PreCompact | Set compaction `custom_instructions` to preserve NOTEBOOK §1; record the compaction |
| UserPromptSubmit | Re-inject NOTEBOOK §1 if a compaction was recorded since the last turn |
| Stop | `final_report` gate plus findings gate plus a skill nudge (at most 1 per session) |

## 8. Research protocol as a state machine

```
PROFILE → HYPOTHESIZE → PRE-REGISTER → TEST → DIAGNOSE → (MOVE → TEST)* → VALIDATE → GATE → REPORT
```

| Transition | Enforced by (E) |
|---|---|
| → PRE-REGISTER | Current `data_qc` grade exists for the instrument, timeframe and window (06 §5) |
| → TEST | `create_experiment` refuses without a registered hypothesis (mechanism, counterparty, persistence, falsifier, null, kill criterion, alternatives) (11 §2) |
| MOVE | Structural edits only. Numbers come from sweeps (P1); every submission is counted (INV-1) |
| → GATE (G3) | Null chosen and logged (Set J); dossier complete before the vault (11 §5) |
| → REPORT | Validator (§10) |

Phases gate *order*. What happens inside each phase is the agent's decision.

## 9. Sub-agents

| Role | Purpose | Context and brief | Tools | Model and effort |
|---|---|---|---|---|
| **Analyst** | Q&A inside a long session without polluting the main context | Question plus handles | `tbot` read-only analysis | Same as main |
| **Hypothesis worker** | Parallel campaign threads, one per hypothesis, each on its own git branch and Experiment | Shared byte-identical brief (instrument profile, `data_qc`, relevant findings) with the hypothesis last (04 §8) | Full research scope | Same as main |
| **Critic** | Falsification report: which nulls and robustness studies to run, what fails | **Fresh context**; brief = Experiment ledger plus dossier artifacts, never the transcript | Evaluation read tools, `run_study` | Same as main |
| **Citation** | Attach ids to claims; flag uncited numbers before `final_report` | Fork of the parent (shares cache) | Read-only | Same as main |
| **Author** | Mechanical SLv2 or DatasetSpec writing from a structural spec | Spec plus validator | Validator | May be cheaper, **only because the validator checks the output** *(flagged tradeoff, 04 §7)* |
| **Quarantined reader** | Read untrusted text (news, social, web, filings) | Text only | **No tools, no network**; returns typed extractions (entity, event, sentiment, novelty, quote spans) | Same as main |
| **Reviewer** | `skill_verify` review (13 §4) | Only the skill, its tests and task descriptions; never the author's transcript | Replay harness | Same as main |
| **Curator** | Sleep-time consolidation: playbook deltas, memory hygiene | Structured deltas only (findings, skill bill of materials), never transcripts (12 §8) | Memory and registry propose APIs | Same; Batch where offline (04 §8) |

**Caps:**
- sub-agent depth 1;
- concurrent sub-agents ≤ the project's job-queue share;
- each sub-agent budget is carved from the session's.

Every run a sub-agent causes goes through the same Experiments and counters. **Scaling
rules in the prompt:**
- 1 agent for lookups;
- 2–4 for comparisons;
- hypothesis workers only for independent hypotheses.

## 10. Termination: `final_report`

The agent writes `reports/final_report.json` and submits it with
`tbot report submit reports/final_report.json`. The platform validates it and returns
`rep_…`. The Stop hook requires a `rep_…` for this session.

```jsonc
final_report {
  answer: "one paragraph",
  outcome: "answered | vaulted | failed_gate_n | inconclusive | aborted",
  claims:     [{ text, value?, evidence: ["exp_…","job_…","art_…","fnd_…"], class }],
  candidates: [{ strategy_ref, experiment_id, gate_reached, trials, effective_n, dossier_ref }],
  rejected:   [{ hypothesis_id, reason, evidence }],
  caveats:    [...],          // data grade, history depth, fill policy, contamination, survivorship
  exploration_ledger_ref, next_steps: [...]
}
```

**Validation (E):**
- schema is strict;
- every id exists and belongs to the project;
- every `value` matches the cited artifact within its declared rounding;
- `class` matches the source (a `result` needs a Set J artifact);
- a candidate past G3 needs a complete dossier (11 §5);
- every hypothesis in the plan has a status.

A deterministic checker runs first. An LLM citation pass examines only the claims the
checker flags.

## 11. Durability

- State lives in the workspace (git), the platform registries, the job service and
  memory, never only in a process.
- **Resume on restart:** the orchestrator restarts `agent-host` with SDK session resume
  where available *(verify)*. Otherwise it starts a new session whose bootstrap re-derives
  state from disk and registries.
- **Idempotency:** job submission is keyed by manifest hash (05 §3). Re-submitting after a
  resume or compaction returns the existing job and never creates a new trial (guards
  against duplicate-trial hazard H13, 04 §10).

## 12. Security

- **Token scopes** (minted per session, bound to one project):
  - `research:data.read` (clipped at the cutoff);
  - `research:features`;
  - `research:jobs`;
  - `research:experiments`;
  - `research:models.train`;
  - `research:memory`;
  - `research:skills.propose`;
  - `research:reports`;
  - `research:proposals.create`.

  **Never granted:** `trade:*`, `automation:*`, `model.alias:*`, `skills.admit`,
  `data.holdout`. The vault gate and admission services hold those.
- **Untrusted text** reaches only the quarantined reader (B-11). Text endpoints require a
  reader-only scope.
- **Skills** follow 13 §6 security rules: no capability grants, signed bundles, quarantine.
- **Audit:** every tool call, job submission and data read is logged against session, user
  and project.

## 13. Budgets

| Unit | Enforced at | Behaviour at the limit |
|---|---|---|
| Model dollars (session, project, day) | LLM proxy | 80%: checkpoint and `ask_user` for more. 100%: proxy refuses; the session wraps up |
| Compute (job-seconds, GPU-seconds) | Job service | Queue refuses new jobs; running jobs finish |
| Wall clock | Orchestrator | Soft stop → report |
| Backtest trials | Evaluation service | Visible, not capped by default; campaign budgets set caps |

## 14. Retirement of the current loop

Once RT acceptance passes:
- remove `crates/api/src/agent/{driver,prompt}.rs` and the `/agent` run loop;
- remove the MCP draft-builder tools.

Keep `crates/llm` for `LlmInference` nodes and model adapters. Keep `crates/mcp-server`
as a thin client (14). Until then, apply the audit's fix to `driver.rs`: elide only in
batches above the threshold, have placeholders carry `art_…` ids, and count all usage
fields (research/R4 §7.15).

## 15. Requirements

| ID | Requirement | Pri |
|---|---|---|
| RT-01 | The agent MUST run in a per-project rootless container from a versioned image, with a persistent `/workspace` volume | MUST |
| RT-02 | The container MUST have network egress only to the platform API, LLM proxy and package mirror | MUST |
| RT-03 | No long-lived secret MAY be present in the container. Model calls go through the LLM proxy with a session token | MUST |
| RT-04 | The orchestrator MUST create, resume, stop and snapshot sessions, and MUST resume running sessions after a platform restart | MUST |
| RT-05 | Session tokens MUST carry only the §12 research scopes and be bound to one project | MUST |
| RT-06 | Every invariant in §2's table MUST be enforced by the listed service, independent of prompts and hooks | MUST |
| RT-07 | The workspace MUST follow §5, with `.claude/**` mounted read-only | MUST |
| RT-08 | New projects MUST run an initializer session; every session MUST start with `bootstrap.sh` | MUST |
| RT-09 | Hooks in §7 MUST be installed and not editable by the agent | MUST |
| RT-10 | Steering messages MUST reach the agent at the next turn boundary without restarting the session | MUST |
| RT-11 | `ask_user` MUST support options, a default and a timeout, and render as an approval card | MUST |
| RT-12 | Sessions MUST end with a platform-validated `final_report` (§10) or an explicit abort record | MUST |
| RT-13 | Sub-agent roles in §9 MUST exist with the stated context isolation. The Critic MUST start fresh; the quarantined reader MUST have no tools or network | MUST |
| RT-14 | Sub-agent depth MUST be ≤ 1 and concurrency ≤ the project queue share | MUST |
| RT-15 | Budgets in §13 MUST be enforced at the listed layer, with 80% and 100% behaviour | MUST |
| RT-16 | All agent events MUST be persisted and streamed to the UI over SSE | MUST |
| RT-17 | Job submission from the agent MUST be idempotent by manifest hash | MUST |
| RT-18 | The protocol transitions in §8 MUST be enforced by the listed endpoints | MUST |
| RT-19 | The model and effort MUST be pinned per project version and MUST NOT change inside a session (04 §7) | MUST |
| RT-20 | The current driver MUST receive the interim elision and usage fix until it is retired (§14) | SHOULD |
| RT-21 | The orchestrator SHOULD support cloning a project (workspace plus memory scope, new cutoff) | SHOULD |
| RT-22 | The LLM proxy MUST be able to rewrite outbound `cache_control.ttl` (5m ↔ 1h, `extended-cache-ttl` beta) per project. The Agent SDK exposes no cache-TTL option (AGENT-001 §23), so the proxy is the only place this can be set **(E)** | MUST |
| RT-23 | The proxy MUST support both credential kinds of D-16 — Anthropic API key (primary) and Claude subscription (fallback) — selecting per project and pinning the kind for the life of a session **(E)** | MUST |

## 16. Acceptance criteria

1. A new project runs its initializer and a session. The session reads bars through
   `tbot`, submits a backtest job, waits for it through an event, and submits a
   validated `final_report`. No secret appears in the container environment or filesystem.
2. Killing the platform mid-session resumes the session. Re-submitting the same job
   manifest returns the same job, and the trial count is unchanged.
3. From the container, requests to a non-allowlisted host fail, writing to
   `.claude/skills/` fails, and a data read past the cutoff returns clipped data with the
   cutoff in the manifest.
4. A steering message changes the agent's next action within one turn. `ask_user` with no
   answer returns the default at timeout.
5. A `final_report` containing a number that doesn't match its cited artifact is rejected,
   with the offending claim named.
6. Hitting 100% of the dollar budget makes the proxy refuse further model calls, and the
   session records an abort.

## 17. Open questions

1. **Python vs TypeScript Agent SDK** for `agent-host`. Recommended: Python, the same
   language as `tbot` and the trainer.
2. **Session resume semantics** of the pinned SDK version (native resume vs re-bootstrap).
3. **Proxy compatibility:** confirm the SDK honours a custom base URL and auth token for
   all model calls, including sub-agents and compaction.
4. **Container runtime** beyond the dev box: Docker Desktop now, Podman or Kubernetes
   later?
