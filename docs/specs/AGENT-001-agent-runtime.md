# AGENT-001: Agent Runtime and Harness

**Status:** Proposed (Phase 0 contract; not implemented)
**Version:** 0.1
**ADR(s):** ADR-0024 (runtime), ADR-0025 (platform-side enforcement); keeps ADR-0023 P1
and Set J INV-1/2/3
**Derived from:** BS-007 [02_AGENT](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/02_AGENT.MD),
[03_RUNTIME](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/03_RUNTIME.MD),
[04_CONTEXT_AND_COST](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/04_CONTEXT_AND_COST.MD)
**Plan set:** L (Agent foundation)
**Crates and apps:**
- new `crates/agent-orchestrator`;
- new `crates/llm-proxy`;
- new `apps/agent-host` (Python);
- new `infra/agent-image/`;
- `crates/api` (auth scopes, routes);
- `migrations/0036_*`;
- retires `crates/api/src/agent/{driver,prompt}.rs`.

**Depends on:**
- COMP-005 (jobs and artifacts);
- DATA-005 (Data API, cutoff, Desk);
- AGENT-002 (`tbot` SDK/CLI);
- AGENT-003 (skills materialisation);
- AGENT-004 (eval suite).

> **Verify in the Phase 0 spike:** every Claude Agent SDK identifier in this spec
> (`ClaudeSDKClient`, `ClaudeAgentOptions`, hook names, in-process MCP tools, base-URL
> and auth-token env vars, cache TTL options, sub-agent caps, session resume) must be
> checked against the SDK version we pin. They are marked *(verify)*. Where the SDK
> differs, this spec adapts the mechanism and keeps the requirement.

---

## 1. Purpose

Run the platform's single quant research agent as a coding agent in a sandboxed
container per research project. It works in files and code, reaches the platform only
through `/api/*`, and runs for hours without polling or losing state. Every research
invariant is enforced by platform services, not by the prompt.

## 2. Scope and non-goals

**In scope:**
- research projects and sessions;
- the container image and sandbox;
- the Rust session orchestrator;
- `agent-host` (the SDK process);
- the LLM proxy;
- scoped session tokens;
- workspace and bootstrap contracts;
- hooks;
- sub-agent definitions;
- steering, `ask_user` and approvals;
- `final_report` submission and validation;
- budgets;
- the event stream;
- resume;
- retirement of the current loop.

**Non-goals:**
- tool and CLI surface (AGENT-002);
- skills lifecycle beyond materialisation hooks (AGENT-003);
- job internals (COMP-005);
- data semantics (DATA-005);
- UI layout (COMP-006);
- eval suites (AGENT-004).

The agent never gets order, automation, alias-promotion or holdout capability.

## 3. Governing rules

| Rule | Statement | Enforced by |
|---|---|---|
| R1 | Invariants are enforced by services the agent's token can't bypass | §6 scopes, DATA-005 cutoff, COMP-005 counting, §15 validator |
| R2 | No long-lived secret in the container | §9 LLM proxy |
| R3 | Model, effort and system core are pinned per project version and never change within a session | §8.2 |
| R4 | The tool surface never changes within a session | §8.2; AGENT-002 |
| R5 | State lives on disk, in platform registries and in the job service, never only in a process | §10, §12 |
| R6 | Never trade model capability for tokens; remove waste instead | §17; BS-007 04 |

## 4. Architecture

```
UI ──SSE/REST──► crates/api ──► crates/agent-orchestrator ──docker API──► project container
                     │                  │   ▲ bridge (WebSocket)            ├─ apps/agent-host (Python, Agent SDK)
                     │                  │   └────────────────────────────── ├─ tbot SDK/CLI, tbot_features
                     │                  ▼                                   └─ /workspace (volume)
                     │            Postgres: research_projects, agent_sessions,          │
                     │            agent_events, approval_requests, llm_usage             │ HTTPS
                     ▼                                                                   ▼
                crates/llm-proxy  ◄──────────── model calls (/llm/v1/messages) ────────────┘
                     │ injects key (credentials store, 0034), budgets, telemetry
                     ▼
                  Anthropic API
```

| Component | Location | Responsibility |
|---|---|---|
| Orchestrator | `crates/agent-orchestrator` (mounted in `apps/platform`) | Projects, sessions, container lifecycle, token minting, skill materialisation call-out (AGENT-003), event bridge, steering, approvals, budgets (wall clock), resume |
| LLM proxy | `crates/llm-proxy` (mounted at `/llm`) | Session-token auth, key injection, dollar budgets, usage telemetry, streaming passthrough |
| agent-host | `apps/agent-host` (Python package baked into the image) | Runs the Agent SDK client; bridges events; receives steering; implements the in-process `ask_user` tool and hook callbacks |
| Image | `infra/agent-image/Dockerfile` | Python 3.11, SDK, `tbot`, `tbot_features` wheel, research stack |
| Auth scopes | `crates/api/src/auth` | Scope-carrying service sessions; `require_scope` middleware |

## 5. Domain model and storage (`migrations/0036_agent_projects_sessions.sql`)

```sql
CREATE TABLE research_projects (
  project_id        UUID PRIMARY KEY,
  user_id           UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
  kind              TEXT NOT NULL CHECK (kind IN ('research','desk')),  -- one 'desk' per user
  name              TEXT NOT NULL,
  goal              TEXT,
  instruments       TEXT[] NOT NULL DEFAULT '{}',
  research_cutoff   TIMESTAMPTZ,          -- NULL for desk (= now); immutable once an Experiment exists (DATA-005)
  holdout_len       INTERVAL,
  project_version   INT  NOT NULL DEFAULT 1,  -- bumps pin model/effort/core/skill selection
  model             TEXT NOT NULL,        -- pinned, e.g. claude-opus-5
  effort            TEXT NOT NULL DEFAULT 'high',
  image_version     TEXT NOT NULL,
  budget_usd_day    NUMERIC(12,2), budget_usd_total NUMERIC(12,2),
  budget_compute_s  BIGINT, tier TEXT NOT NULL DEFAULT 'standard',
  workspace_volume  TEXT NOT NULL,
  status            TEXT NOT NULL DEFAULT 'active',   -- active | archived
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX one_desk_per_user ON research_projects(user_id) WHERE kind = 'desk';

CREATE TABLE agent_sessions (
  session_id     UUID PRIMARY KEY,
  project_id     UUID NOT NULL REFERENCES research_projects ON DELETE CASCADE,
  user_id        UUID NOT NULL,
  state          TEXT NOT NULL,   -- §11 state machine
  is_initializer BOOLEAN NOT NULL DEFAULT false,
  project_version INT NOT NULL,
  sdk_session_ref TEXT,           -- SDK resume handle (verify)
  container_id   TEXT,
  report_id      TEXT,            -- rep_… once validated
  abort_reason   TEXT,
  spend_usd      NUMERIC(12,4) NOT NULL DEFAULT 0,
  started_at TIMESTAMPTZ, ended_at TIMESTAMPTZ, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE agent_events (
  id BIGSERIAL PRIMARY KEY, session_id UUID NOT NULL REFERENCES agent_sessions ON DELETE CASCADE,
  seq INT NOT NULL, kind TEXT NOT NULL, payload JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(), UNIQUE (session_id, seq)
);

CREATE TABLE approval_requests (
  approval_id UUID PRIMARY KEY, project_id UUID NOT NULL, session_id UUID,
  kind TEXT NOT NULL,   -- ask_user | plan | budget | qc_waiver | model_promotion | skill_promotion | paper_deployment
  payload JSONB NOT NULL, options JSONB, default_option TEXT, timeout_at TIMESTAMPTZ,
  state TEXT NOT NULL DEFAULT 'pending',  -- pending | answered | defaulted | cancelled
  answer JSONB, answered_by UUID, answered_at TIMESTAMPTZ, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE llm_usage (
  id BIGSERIAL PRIMARY KEY, session_id UUID, project_id UUID, user_id UUID NOT NULL,
  role TEXT NOT NULL,         -- main | subagent:<name> | curator | replay | eval
  model TEXT NOT NULL, effort TEXT, cache_ttl TEXT,
  input_tokens INT, cache_write_5m INT, cache_write_1h INT, cache_read INT, output_tokens INT,
  compaction_iterations INT, stop_reason TEXT, gap_ms BIGINT, prefix_hashes JSONB,
  cost_usd NUMERIC(12,6) NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE sessions ADD COLUMN IF NOT EXISTS scopes TEXT[] NOT NULL DEFAULT '{}';
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS project_id UUID;
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS agent_session_id UUID;
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ;
```

`agent_runs` and `agent_messages` (0035) stay read-only for history and are dropped
after retirement (§20).

## 6. Session tokens and scopes

- The orchestrator mints a `sessions` row with `kind='service'`, `project_id`,
  `agent_session_id`, `scopes`, and `expires_at = now + 24h`. It is rotated on resume
  and revoked on stop.
- **Research-session scopes:**
  - `research:data.read`;
  - `research:features`;
  - `research:jobs`;
  - `research:experiments`;
  - `research:models.train`;
  - `research:memory`;
  - `research:skills.propose`;
  - `research:reports`;
  - `research:proposals.create`;
  - `research:artifacts`;
  - `llm:proxy`.
- **The reader sub-agent's token** (§13) gets `research:text.reader` and `llm:proxy`
  only.
- **Never minted for agent sessions:** `trade:*`, `automation:*`, `model.alias:*`,
  `skills.admit`, `data.holdout`.
- **Middleware:** `require_scope(scope)` is applied per route. Every request is resolved
  to `(user_id, project_id, agent_session_id)`. Project-bound tokens can reach only their
  own project's resources, returning `403 scope_denied` or `404` otherwise.
- **Web sessions** (humans) keep full user rights. Scopes apply to service sessions only.

## 7. Container and sandbox

| Item | Specification |
|---|---|
| Image | `tbot-agent:<semver>` from `infra/agent-image/Dockerfile`: `python:3.11-slim` + Node runtime if the SDK requires it *(verify)* + `claude-agent-sdk` (pinned) + `tbot` + `tbot_features` wheel + polars, numpy, scipy, statsmodels, arch, scikit-learn, lightgbm, xgboost, torch (CPU), duckdb, plotly, pyarrow. No compilers beyond what the wheels need |
| User | Non-root UID 10001. `--cap-drop ALL`, `--security-opt no-new-privileges`, `--read-only` root filesystem |
| Writable | `/workspace` (named volume `tbot-ws-<project_id>`), `/tmp` (tmpfs, size-limited), `/home/agent/.cache` (tmpfs) |
| Limits | Per tier: CPUs, memory, PIDs, `/tmp` size. Defaults: standard = 4 CPU, 8 GiB, 512 PIDs |
| Network | Attached only to `tbot-agent-net`, a Docker network created with `internal: true` (no default route). Reachable: the platform's API and proxy endpoint (the platform container is dual-homed on this network) and a `devpi` PyPI mirror on the same network. Nothing else resolves or routes |
| Environment | `TBOT_API_URL`, `TBOT_TOKEN` (session token), `TBOT_PROJECT_ID`, `TBOT_SESSION_ID`, `ANTHROPIC_BASE_URL=<platform>/llm`, `ANTHROPIC_AUTH_TOKEN=<session token>` *(verify variable names)*. No provider key |
| Lifecycle | Created on first session; stopped after `idle_stop_after` (default 15 min) without an active session; the volume persists; removed on project archive (volume snapshot kept) |
| Snapshots | On session stop: `git commit` of the workspace (by `agent-host`), plus a volume snapshot where the backend supports it |

## 8. agent-host

### 8.1 Responsibilities

1. Connect to the orchestrator bridge (`wss://<platform>/agent/bridge?session=…`,
   session token).
2. Build the SDK client (§8.2) and run the session. Stream every SDK message to the
   bridge as an `agent_event`.
3. Receive steering messages and inject them as user turns at the next boundary, through
   the SDK's streaming input *(verify)*.
4. Implement the in-process tool `ask_user(question, options, default, timeout_s)`. It
   posts an `approval_requests` row through the orchestrator, blocks, and returns the
   answer, or the default at timeout.
5. Implement hook callbacks (§12), calling platform endpoints where needed.
6. On stop: commit the workspace, flush events, exit with a status.

### 8.2 SDK configuration (pinned per project version)

| Option | Value |
|---|---|
| System prompt | A stable core: identity, behaviour contract (BS-007 02 §4), protocol summary, workspace map pointer. No dynamic values. Dynamic context (bootstrap output, time) enters as the first user message (e.g. `excludeDynamicSections` *(verify)*) |
| `cwd` | `/workspace` |
| Settings source | Project settings from the read-only `/workspace/.claude/settings.json` (hooks) *(verify loader)* |
| Tools | SDK built-ins (Read, Write, Edit, Bash, Grep, Glob, Task/sub-agents, Skill) + in-process MCP server exposing `ask_user` only. Web fetch/search disabled |
| Model / effort | `research_projects.model` / `.effort` (≥ high) |
| Cache TTL | 1 h for the main loop; per-role for sub-agents *(verify option names)* |
| Compaction | Server-side, same model; instructions from CLAUDE.md (§10.2) |
| Sub-agents | From read-only `/workspace/.claude/agents/*.md` (§13); depth 1; concurrency cap *(verify env/option)* |
| Budget | `max_budget_usd` per session as belt-and-braces *(verify)*. The proxy is authoritative (§9) |

## 9. LLM proxy (`crates/llm-proxy`, route prefix `/llm`)

- **Surface:** `POST /llm/v1/messages` (Anthropic Messages API passthrough, streaming
  and non-streaming), plus `POST /llm/v1/messages/count_tokens`. Other paths get `404`.
- **Auth:** bearer session token with `llm:proxy`. The user's provider key is loaded
  from the credential store (0034) and injected. The token is never forwarded
  upstream.
- **Pre-flight budget check:** estimated maximum cost (input estimate + `max_tokens` ×
  output price) against the remaining session, project and day budgets. If exceeded,
  return `402 budget_exhausted` with `{scope, limit_usd, spent_usd}`. The 80% threshold
  emits a `budget` event and an approval request (`kind='budget'`).
- **Post-flight:** parse `usage`, including cache fields and compaction `iterations`.
  Compute cost from a versioned price table (`config/llm_prices.toml`). Insert
  `llm_usage`. Update `agent_sessions.spend_usd`.
- **Telemetry fields:** as in the `llm_usage` table (§5). The `role` is taken from a
  header set by `agent-host` per sub-agent (`X-Tbot-Role`) *(verify header
  propagation through the SDK; otherwise infer from SDK events)*.
- **Streaming:** pass through server-sent chunks unchanged, and account on stream end.
- **Headers:** forward cache-diagnosis and beta headers the SDK sets. Strip client auth.
- **Failure:** upstream errors pass through with their status. Proxy errors use the
  `{code, message, fix}` envelope.

## 10. Workspace contract

### 10.1 Layout

```
/workspace/
  CLAUDE.md  NOTEBOOK.md  RESEARCH_PLAN.json  PROGRESS.md  bootstrap.sh
  research/ strategies/ models/ datasets/ reports/ skill-drafts/ data/ (git-ignored)
  .claude/  (read-only bind mount: settings.json, skills/, agents/)
  .gitignore  (data/, /tmp spill)
```

Created by the orchestrator from a template on project creation, then committed
(`init: project <id>`).

### 10.2 `CLAUDE.md` (generated per project version, byte-stable)

Sections:
1. project identity (name, kind, instruments, cutoff policy wording, not values);
2. research protocol (§14);
3. workspace map;
4. rules (numbers from tools; Set J for claims; P1; pre-registration; the Desk rule);
5. **compaction instructions**, which preserve:
   - hypothesis ids and statuses;
   - experiment, job and artifact ids;
   - trials and effective N;
   - gate states;
   - rejected ideas and reasons;
   - the cutoff and budget state;
   - open questions;
   - the current step;
   - every number with its n, SE and handle.

No timestamps or per-session values.

### 10.3 `NOTEBOOK.md`

`## 1. Current state` (≤ 2,000 tokens: goal, plan step, active hypotheses, candidates,
blockers) is followed by free sections. The PostCompact hook re-injects §1. A PreToolUse
check warns when §1 exceeds its budget.

### 10.4 `RESEARCH_PLAN.json` (JSON Schema in `apps/agent-host/schemas/research_plan.json`)

```json
{ "project_id": "…", "items": [
  { "id": "H-001", "hypothesis_ref": "hyp_…|null", "archetype": "trend.ema_cross",
    "statement": "…", "status": "untested|running|rejected|supported|vaulted",
    "experiment_ids": [], "created_at": "…", "updated_at": "…" } ] }
```

Items may be added or have their status changed, never deleted. The Stop hook diffs the
file against git history and rejects deletions. The platform hypothesis registry
(BACKTEST_SUITE_CORE_SPEC v2) is authoritative, and `bootstrap.sh` prints drift.

### 10.5 `bootstrap.sh` output contract (≤ 40 lines)

```
project <name> (<kind>) · version <n> · model <m>/<effort>
cutoff <iso|now> · holdout <len> · budget $<spent>/<limit> today, compute <s>/<limit>
platform: ok · data: <instr> <tf> <first>..<last> qc=<A-D> gaps=<n> …
jobs pending: <n> (<id kind state> …up to 5)
plan: <n> items (<untested>/<running>/<rejected>/<supported>/<vaulted>) · drift: none|<ids>
skills: <n> materialised (glossary: skill-finder)
last session: <id> <outcome> <report_id|abort>
```

## 11. Session state machine

```
creating → starting → running ⇄ waiting_input → stopping → stopped
                         │  ▲                               │
                         ▼  │                               ▼
                      suspended (platform restart) → resuming → running
any → failed (unrecoverable; abort_reason set)
```

- **creating:** row inserted, token minted, skills materialised (AGENT-003), container
  ensured.
- **starting:** `agent-host` connected. A SessionStart hook runs `bootstrap.sh`.
- **running:** the SDK loop is active. **waiting_input:** an `ask_user` or plan
  approval is pending (the loop is blocked in the tool).
- **stopping:** the Stop hook is satisfied (§12) or a user stop was requested; the
  workspace is committed.
- **suspended/resuming:** on platform boot, every session in
  `running | waiting_input | starting` goes to `suspended`, then `resuming`. The
  orchestrator restarts `agent-host` with the SDK resume handle *(verify)* or, if resume
  is unavailable, a new session that re-bootstraps from disk and registries. Pending jobs
  continue in COMP-005.
- **The initializer session** (the first in a project): the system core instructs it to
  expand the goal into `RESEARCH_PLAN.json` items, check `bootstrap.sh`, and commit
  `init: plan`.

## 12. Hooks (`.claude/settings.json`, generated, read-only)

| Event | Matcher | Action | On hook failure |
|---|---|---|---|
| SessionStart | * | Run `bootstrap.sh`; inject its output as context | Session → failed (the platform is unreachable) |
| PreToolUse | Write, Edit | Deny paths under `/workspace/.claude/` and outside `/workspace`, `/tmp` | Deny |
| PreToolUse | Bash | Deny commands matching the policy list (`docker`, `sudo`, secret paths, raw `curl`/`wget` to hosts other than `$TBOT_API_URL`); deny `tbot` subcommands outside the token scopes (fast feedback; the platform re-checks) | Deny |
| PreToolUse | Read | Duplicate-read guard: same path and hash as a read since the last compaction → return a short pointer ("unchanged since turn N; use offset/limit for a slice") | Allow |
| PostToolUse | * | Output guard: tool output > 8,000 tokens → write to `/tmp/out/<id>.txt` and replace it with a ≤ 300-token summary plus the path. Emit a usage event | Pass through |
| PostCompact *(verify availability; else SessionStart-on-compact)* | * | Re-inject NOTEBOOK §1 | — |
| Stop | * | Require: (a) `reports/final_report.json` submitted and validated (`rep_…` stored on the session), or an explicit abort record; (b) a finding per hypothesis moved to rejected/supported this session (`tbot memory` check); (c) a clean git commit; (d) at most one skill-proposal nudge (AGENT-003 triggers). If unmet, block the stop with a message listing what's missing, at most 3 times; then allow the stop and record `abort_reason='stop_gate_unmet'` | Allow + record |

The hook implementation lives in `apps/agent-host/hooks/` (Python callbacks or scripts).
Hooks are data in a read-only mount. The agent can't edit them.

## 13. Sub-agents (`.claude/agents/*.md`, read-only)

| Name | Brief and context | Tools | Model | Notes |
|---|---|---|---|---|
| `analyst` | Question + handles | Read, Bash (`tbot analysis`, `tbot data` read-only) | project model | Answer card with cited numbers |
| `hypothesis-worker` | Shared byte-identical brief file `reports/briefs/<campaign>.md`, then the hypothesis | Full | project model | Own git branch and Experiment; started staggered |
| `critic` | Fresh; brief = Experiment ledger + dossier handles only | Read, Bash (`tbot exp …` read-only + `study`) | project model | Never receives the transcript |
| `citation` | Fork of parent | Read | project model | Runs before `tbot report submit` |
| `author` | Structural spec + validator | Read, Write (`strategies/`, `datasets/`), Bash (`tbot strategy validate`, `tbot datasets validate`) | project model, or a cheaper model **only with the validator as checker** (flagged tradeoff; AGENT-004 non-inferiority) | — |
| `reader` | Text only | **None**; no Bash, no network. Runs under a separate token with only `research:text.reader` via the orchestrator's text-fetch step | project model | Returns typed extractions JSON |
| `reviewer` | Skill bundle, tests, task descriptions only | Replay harness commands | project model | Used by `skill_verify` (AGENT-003) |
| `curator` | Structured deltas only | `tbot memory`, `tbot skills propose/patch` | project model; Batch offline | Token cap per run |

**Caps:** depth 1; concurrency ≤ `min(tier cap, job-queue share)`. Scaling guidance is
in the system core: 1 agent for lookups, 2–4 for comparisons, workers only for
independent hypotheses.

**Reader flow:** the main agent calls `tbot text request "<query>" --as-of …`. The
orchestrator fetches the text with the reader token and spawns the `reader` sub-agent
with that text as its only input. Its JSON extractions are returned to the main agent as
data (`artifact` plus summary).

## 14. Research protocol enforcement (references)

| Transition | Endpoint that enforces it | Spec |
|---|---|---|
| Pre-register requires `data_qc` ≥ C (C needs an approved waiver) | `POST /api/hypotheses`, `POST /api/backtest/experiments` | DATA-005 §7, BACKTEST_SUITE_CORE_SPEC v2 |
| Test requires a registered hypothesis | `POST /api/backtest/experiments` | BACKTEST_SUITE_CORE_SPEC v2 |
| Every submission counted; idempotent | `POST /api/jobs` | COMP-005 §4 |
| No live or post-cutoff data in research projects | Data API | DATA-005 §4 |
| Dossier before the vault | `…/funnel/advance` to G4 | BACKTEST_SUITE_CORE_SPEC v2 |

## 15. `final_report`

**Submission:** `POST /api/reports` (CLI `tbot report submit <file>`), scope
`research:reports`.

**Schema** (`schemas/final_report.v1.json`):

```jsonc
{ "schema": "final_report.v1", "session_id": "…", "project_id": "…",
  "answer": "string ≤ 1200 chars",
  "outcome": "answered|vaulted|failed_gate_0|…|failed_gate_4|inconclusive|aborted",
  "claims": [{ "text": "…", "value": 0.0, "unit": "…", "rounding": 2,
               "evidence": ["exp_…|job_…|art_…|fnd_…"], "class": "fact|estimate|exploration|result|model_output" }],
  "candidates": [{ "strategy_ref": "…", "experiment_id": "…", "gate_reached": "G0..G4",
                   "trials": 0, "effective_n": 0.0, "dossier_ref": "art_…|null" }],
  "rejected": [{ "hypothesis_id": "…", "reason": "…", "evidence": ["…"] }],
  "caveats": ["…"], "exploration_ledger_ref": "art_…", "next_steps": ["…"] }
```

**Validation** (deterministic first, in order):
1. The schema is strict.
2. Every evidence id exists and belongs to the project.
3. For each claim with `value`, the value resolves from the cited artifact's manifest
   or metrics and matches within `rounding`.
4. `class=result` requires a Set J artifact (study, experiment, gate verdict).
5. A candidate with `gate_reached ≥ G3` requires `dossier_ref` with all 12 parts
   (BACKTEST_SUITE_CORE_SPEC v2).
6. Every `RESEARCH_PLAN.json` item touched this session has a terminal or running
   status.
7. Numeric tokens in `answer` and `text` without a claim `value` are flagged.

Flagged claims then get an LLM citation check (proxy, role `citation`). **Response:**
`201 {report_id: "rep_…"}` or `422 {errors: [{path, rule, message, fix}]}`.

## 16. Steering, `ask_user`, approvals

| Endpoint | Purpose |
|---|---|
| `POST /api/agent/sessions/{id}/steer` `{text}` | Queue a steering message; delivered at the next turn boundary |
| `POST /api/agent/sessions/{id}/interrupt` | Interrupt the current turn (SDK interrupt *(verify)*) |
| `POST /api/agent/sessions/{id}/stop` | Graceful stop through the Stop gate |
| `GET /api/approvals?state=pending` | Approvals inbox (all kinds) |
| `POST /api/approvals/{id}/answer` `{option|value}` | Answer; unblocks `ask_user` or applies an approved action |

- **Plan approval:** `ask_user` with `kind='plan'` and the per-project auto-approve
  timeout (default 120 s; 0 = auto).
- **Timeouts** resolve to the default (`state='defaulted'`).

## 17. Budgets and cost controls

| Unit | Where enforced | 80% | 100% |
|---|---|---|---|
| Model dollars (session, project total, project/day) | LLM proxy | `budget` event + approval request | `402`; agent-host records an abort and stops |
| Compute seconds and GPU seconds | Job service (COMP-005) | event | New submissions refused (`409 budget_exhausted`) |
| Wall clock (session) | Orchestrator | event | Stop gate |

**Context and cost rules** (BS-007 04, normative here):
- a 1-hour cache TTL on the main loop;
- a stable prefix (R3/R4);
- event-driven waits (COMP-005 §6);
- compact before waits expected to exceed the TTL (system core instruction);
- the output guard and duplicate-read guard (§12);
- same-model compaction.

Telemetry is in §9. Every change to these rules must pass AGENT-004 non-inferiority.

## 18. Event stream

`GET /api/agent/sessions/{id}/events?after_seq=` (SSE). Events are persisted in
`agent_events` and replayable.

| Kind | Payload (abridged) |
|---|---|
| `message` | `{role, text}` |
| `tool_call` / `tool_result` | `{tool, input_summary}` / `{tool, summary, for_user_ref?, spilled_path?}` |
| `subagent_start` / `subagent_end` | `{name, brief_ref}` / `{name, result_summary}` |
| `compaction` | `{before_tokens, after_tokens}` |
| `checkpoint` | `{kind, text, refs}` |
| `approval_request` / `approval_resolved` | `{approval_id, kind, …}` |
| `job_state` | `{job_id, kind, state, summary}` (relayed from COMP-005) |
| `artifact_created`, `report_submitted` | `{handle}` / `{report_id}` |
| `budget` | `{unit, spent, limit, pct}` |
| `skill_notice` | `{skill_id, action: revoked|quarantined}` (appended as a system message in-session) |
| `state` | `{from, to}` |

## 19. REST surface (orchestrator)

| Method and path | Scope | Purpose |
|---|---|---|
| `POST /api/agent/projects` | web | Create a research project (name, goal, instruments, holdout_len, budgets, model, tier) |
| `GET /api/agent/projects`, `GET /api/agent/projects/{id}` | web | List and read (the Desk is auto-created per user) |
| `POST /api/agent/projects/{id}/clone` | web | New project, new cutoff, same memory scope |
| `POST /api/agent/projects/{id}/version` | web | Bump project version (model, effort, image, skill selection) |
| `POST /api/agent/sessions` `{project_id, prompt}` | web | Start a session (a question to the Desk goes to the Desk project) |
| `GET /api/agent/sessions/{id}`, `…/events` | web | Status and stream |
| Steering and approvals | web | §16 |
| `GET /api/agent/usage?project|session` | web | Cost, cache hit rate, dollars per verdict |

## 20. Retirement and migration

1. While Set L is in progress, apply the interim `driver.rs` fix: elide in batches only
   above the threshold; placeholders carry `art_…` ids; budgets count all usage fields.
2. At Set L exit:
   - remove the `/api/agent/runs*` routes, `crates/api/src/agent/{driver,prompt,manager}.rs`
     and the `AgentPage` run loop;
   - keep `agent_runs` and `agent_messages` read-only for 90 days, then drop them
     (migration);
   - remove the MCP draft-builder tools (AGENT-002).
3. `crates/llm` stays for `LlmInference` nodes and backs the proxy's upstream client.

## 21. Security checklist

- A container escape surface review: rootless, caps dropped, read-only root, no Docker
  socket, internal-only network.
- No provider key reachable from the container: environment, filesystem and proxy
  responses are checked in tests.
- Scope tests: every `/api/*` route has a declared scope. A CI test fails on routes
  with no scope annotation.
- Text from news, social and web reaches only the `reader`.
- An audit log of all service-session requests: `(session, route, status, bytes)`.

## 22. Test plan and acceptance

| # | Test | Requirement (BS-007 IDs) |
|---|---|---|
| A1 | New project → initializer → session reads bars via `tbot`, submits a backtest job, waits by event, submits a validated report; no secret in env, filesystem or `/proc` | RT-01…RT-05, RT-08, RT-12 |
| A2 | Kill the platform mid-session → the session resumes; re-submitting the same manifest returns the same job; the trial count is unchanged | RT-04, RT-17 |
| A3 | From the container: a non-allowlisted host fails; a write to `.claude/skills` is denied; a data read past the cutoff is clipped (manifest states it); a live read in a research project is refused | RT-02, RT-07, RT-09, DA-03, DA-15 |
| A4 | A steering message changes the next action within one turn; `ask_user` without an answer returns the default at timeout | RT-10, RT-11 |
| A5 | A report with a mismatched number is rejected with the claim named | RT-12 |
| A6 | Budget at 100% → proxy `402` → abort recorded | RT-15 |
| A7 | 3-hour synthetic build session: cache hit rate ≥ 85%, no unplanned rewrites; a 90-minute job wait has no polling and a post-wait rewrite ≤ 40k tokens | CX-01…CX-05 |
| A8 | A 50k-token tool output is spilled; the model receives ≤ 1k tokens plus the path | CX-06 |
| A9 | Critic sub-agent context contains no transcript; reader sub-agent has no tools | RT-13 |
| A10 | Scope CI: every route annotated; agent token denied on `/api/orders`, automations and alias routes | RT-05 |

## 23. Open items for the Phase 0 spike

1. Confirm SDK identifiers and behaviours marked *(verify)*: base URL and auth token,
   resume, streaming input for steering, PostCompact availability, TTL options,
   sub-agent caps, role header propagation.
2. Measure the container cold start and `agent-host` start time. Target a session start
   under 10 s.
3. Choose the container backend beyond the dev box (Docker Desktop now; Podman or
   Kubernetes later).

## 24. Traceability

This spec implements:
- BS-007 RT-01…RT-21 (except RT-20, which is interim);
- CX-01…CX-06, CX-12 and CX-14 (with AGENT-004 for CX-13);
- B-R1…B-R5.
