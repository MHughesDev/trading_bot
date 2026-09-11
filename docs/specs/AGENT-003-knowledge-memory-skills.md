# AGENT-003: Knowledge, Memory and Skills

**Status:** Proposed (Phase 0 contract; not implemented)
**Version:** 0.1
**ADR(s):** ADR-0028 (skill registry, platform-side admission, glossary as view),
ADR-0025 (scopes), ADR-0030 (jobs, artifacts)
**Derived from:** BS-007 [12_KNOWLEDGE_MEMORY](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/12_KNOWLEDGE_MEMORY.MD),
[13_SKILLS](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/13_SKILLS.MD)
**Plan sets:**
- L: core skills (static), findings write, materialiser v0;
- R-a: registry, dynamic materialiser, propose, verify, project admission;
- R-b: promotion, curator, A/B, semantic memory, move memory, playbook.

**Crates and apps:**
- new `crates/knowledge` (findings, move memory, profiles, skill registry, materialiser);
- `crates/semantic` (Milvus collections);
- `apps/embedder` (switched to a local model);
- `apps/model-inference` (a `/embed` endpoint for the local embedding model);
- `skills/core/` (repo, core skill sources);
- `migrations/0039_*`.

---

## 1. Purpose

- Give the agent memory that compounds across sessions without replacing measurement.
- Give it a library of procedures that it can grow itself, verified by the platform.
- Materialise skills for every user.
- List every admitted skill in the glossary.

## 2. Memory layers (normative summary)

| Layer | Store | Written by | Read by | Embedded |
|---|---|---|---|---|
| L0 core | Repo (system core, CLAUDE.md template) | Releases | Always resident | No |
| L1 reference | **Core skills** (`skills/core/*`) | Repo PRs | Skill loading, glossary, search | Skill bodies (search) |
| L2 findings | Postgres `findings` + Milvus `findings` | `tbot memory add` (agent), validated | `tbot memory search` | `claim` |
| L2m move memory | Postgres `move_memory` | Harness (automatic on moves) | Proposal scoring, veto | No |
| L2p playbook | Skill registry (`kind=playbook`) | Curator deltas → admission | As skills | Via skills |
| L3 profiles | Postgres `instrument_profiles` (+ ClickHouse stats) | `instrument_profile` jobs | `tbot profile` | No |
| L4 text | ClickHouse/Postgres + Milvus `text` | Collectors | Reader only (DATA-005) | Yes |
| L5 user memory | Postgres `user_memory` KV | UI / agent (explicit) | Bootstrap | No |

## 3. Storage (`migrations/0039_knowledge_skills.sql`)

```sql
CREATE TABLE findings (
  finding_id TEXT PRIMARY KEY,                -- 'fnd_' || ULID
  project_id UUID NOT NULL, user_id UUID NOT NULL, session_id UUID,
  claim TEXT NOT NULL CHECK (length(claim) <= 600),
  instrument TEXT, asset_class TEXT, timeframe TEXT, window_start DATE, window_end DATE,
  archetype TEXT, gate_reached TEXT, class TEXT NOT NULL,       -- result | exploration | model_output
  evidence TEXT[] NOT NULL,                                     -- exp_/job_/art_/study_ ids (validated)
  hypothesis_id TEXT, platform_capabilities JSONB NOT NULL,     -- e.g. {features_engine: 2, exits: true, costs: "edge_v1"}
  visibility TEXT NOT NULL DEFAULT 'project',                   -- project | user | global
  tainted BOOLEAN NOT NULL DEFAULT false, supersedes TEXT REFERENCES findings,
  embedding_model TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE move_memory (
  id BIGSERIAL PRIMARY KEY, user_id UUID NOT NULL, project_id UUID,
  parent_archetype TEXT NOT NULL, regime_profile JSONB NOT NULL, parent_gate TEXT NOT NULL,
  move_motif TEXT NOT NULL,                    -- canonical AST-diff motif (FEAT-004)
  residual DOUBLE PRECISION NOT NULL,          -- child sealed metric − base prior
  metric TEXT NOT NULL, experiment_id TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE instrument_profiles (
  instrument TEXT, timeframe TEXT, as_of TIMESTAMPTZ, profile JSONB NOT NULL, job_id TEXT,
  PRIMARY KEY (instrument, timeframe, as_of)
);
CREATE TABLE user_memory (user_id UUID, key TEXT, value JSONB, updated_at TIMESTAMPTZ, PRIMARY KEY (user_id, key));

CREATE TABLE skills (
  skill_id TEXT PRIMARY KEY,                   -- 'skl_' || ULID
  name TEXT NOT NULL, scope TEXT NOT NULL CHECK (scope IN ('core','global','user','project')),
  owner_user_id UUID, project_id UUID, category TEXT NOT NULL, kind TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('provisional','admitted','deprecated','quarantined')),
  current_version TEXT NOT NULL, pinned BOOLEAN NOT NULL DEFAULT false,
  alias_of TEXT REFERENCES skills, updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- name uniqueness: core/global names reserved across all scopes
CREATE UNIQUE INDEX skills_name_global ON skills(name) WHERE scope IN ('core','global');
CREATE UNIQUE INDEX skills_name_scoped ON skills(name, scope, coalesce(owner_user_id, '00000000-0000-0000-0000-000000000000'), coalesce(project_id, '00000000-0000-0000-0000-000000000000'));
CREATE TABLE skill_versions (
  skill_id TEXT REFERENCES skills, version TEXT, bundle_handle TEXT NOT NULL,   -- art_… (skill_bundle)
  content_sha256 TEXT NOT NULL, description TEXT NOT NULL, description_tokens INT NOT NULL,
  spec JSONB NOT NULL,                       -- skill.yaml
  provenance JSONB NOT NULL, review JSONB, signature TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (skill_id, version)
);
CREATE TABLE skill_runs (                     -- bill of materials
  session_id UUID, skill_id TEXT, version TEXT, state TEXT NOT NULL,  -- exposed | activated | excluded
  script_exit_codes INT[], task_outcome TEXT, vote SMALLINT, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE skill_evals (skill_id TEXT, version TEXT, kind TEXT, result JSONB, job_id TEXT, created_at TIMESTAMPTZ DEFAULT now());
CREATE TABLE skill_proposals (
  proposal_id TEXT PRIMARY KEY, session_id UUID, project_id UUID, user_id UUID,
  reason TEXT NOT NULL CHECK (reason IN ('T1','T2','T3','T4')), source_trajectory TEXT NOT NULL,
  target_skill_id TEXT,                       -- set when converted to a patch (§6.3)
  draft_handle TEXT, state TEXT NOT NULL, verify_job TEXT, created_at TIMESTAMPTZ DEFAULT now()
);
CREATE VIEW skill_glossary AS
  SELECT s.skill_id, s.name, s.scope, s.category, s.status, s.kind, s.owner_user_id, s.project_id,
         v.version, v.description
  FROM skills s JOIN skill_versions v ON v.skill_id = s.skill_id AND v.version = s.current_version
  WHERE s.status IN ('provisional','admitted');
```

**Milvus collections** (`crates/semantic`):
- `findings` (vector, finding_id, instrument, timeframe, archetype, gate, class,
  visibility, user_id, project_id, created_at);
- `skills` (vector over the full body, skill_id, version, scope, category, status);
- `text` (existing social/web plus news and filings, with `available_time`).

All three use hybrid dense + BM25 (Milvus ≥ 2.4) and record `embedding_model` and `dim`.

## 4. Local embeddings

- `apps/model-inference` exposes `POST /embed {texts[], model}` for a local embedding
  model. The model is chosen by a 50-query retrieval eval over findings and skill
  bodies; this is an open question.
- `apps/embedder` switches from OpenAI `text-embedding-3-small` to `/embed`.
- **Re-embedding** is a job (`reembed`) on a model change. Vectors are versioned by
  `embedding_model`, and queries target one model version.
- **Egress test:** no embedding request leaves the host (AGENT-004 robustness).

## 5. Findings

- `tbot memory add --claim … --evidence exp_… [--hypothesis …] [--class result]` calls
  `POST /api/memory/findings` (scope `research:memory`). Validation:
  - the evidence ids exist in the project;
  - `class=result` needs a Set J artifact;
  - the claim is ≤ 600 chars;
  - `platform_capabilities` is stamped by the server.

  A finding is `tainted` if the session consumed reader extractions.
- **Search:** `GET /api/memory/findings/search?q&instrument&timeframe&archetype&gate&limit`
  applies metadata filters first, then hybrid retrieval, then a cross-encoder rerank of
  the top 30. The response carries age, a staleness flag and a capability-mismatch flag.
- **Honesty:** findings are never valid evidence for a new Experiment. A hypothesis
  whose `source=memory:<fnd>` on the same instrument and window is flagged a repeated
  trial (BACKTEST_SUITE_CORE_SPEC v2).
- The Stop hook requires a finding for each hypothesis resolved in the session
  (AGENT-001 §12).

## 6. Skills

### 6.1 Core skills (Plan set L)

- Sources live in `skills/core/<category>/<name>/` (SKILL.md, `reference/`, `scripts/`,
  `skill.yaml`), seeded from BS-007 research/R3 and 11 §7:
  - QC rules and pitfalls;
  - the `tbot` how-tos;
  - the `skill-finder` skill (containing the glossary);
  - the `skill-authoring` skill;
  - the playbook.
- They are loaded into the registry at deploy time with `scope=core, status=admitted`,
  and signed by the release.
- **v0 materialiser (Set L):** mounts all core skills plus a glossary generated from the
  `skill_glossary` view.

### 6.2 Materialiser (Set R-a; runs in AGENT-001 §11 `creating`)

**Input:** `(user_id, project_id, project_version)`.

1. Resolve the visible set from `skill_glossary`: all `core`; all `global`; the user's
   `user`-scope skills; the project's `project`-scope skills. Lower scopes can't shadow
   core or global names (unique index).
2. **Resident set:**
   - core (always), then pinned global, then the project's **frozen selection**
     (`project_skill_selection`, recomputed only on a project version bump from
     `skill_runs` helpfulness), up to **40** in total;
   - order: tier, then name;
   - descriptions are verbatim from the admitted version (≤ 100 tokens each, enforced at
     admission).
3. Write the resident skills to the read-only mount `.claude/skills/<name>/`. Write
   `GLOSSARY.md` (category tree; one line per visible skill: name, purpose, scope,
   status, version) inside `skill-finder/`, where it loads on demand.
4. Everything else is reachable through `tbot skills search` (hybrid over full bodies,
   with rerank).
5. Record a `skill_runs` row with `state='exposed'` for each resident skill.
6. Verify each bundle's sha256 and signature. A mismatch aborts materialisation.

**Mid-session revocation:** the orchestrator emits a `skill_notice` event, which is
appended as a system message. The system prompt is never edited. The skill is removed
at the next session start.

### 6.3 Proposal and verification (Set R-a)

- **Triggers** (in the system core, nudged by the Stop hook at most once per session):
  - T1 repetition: the same procedure 3 times in a project, or in 2 projects, detected
    from workspace script AST and hash similarity in git history;
  - T2 hard-won and verified: ≥ 15 tool calls or an error-then-fix, **plus** a
    platform-verified outcome;
  - T3 pitfall: a tagged platform rejection;
  - T4 human request.

  **Never:** speculative proposals, one-offs, wrappers of a single tool, content with
  findings, symbols, dates or tuned values, untestable procedures, or proposals whose
  only source was untrusted text.
- **Propose:** `tbot skills propose --reason T2 --from <session|art>` calls
  `POST /api/skills/proposals`.
  - It runs a nearest-neighbour search over full bodies.
  - If similarity ≥ `skills.patch_threshold` (default 0.85, tuned by eval), the proposal
    becomes a **patch** of `target_skill_id`.
- **Draft:** in `/workspace/skill-drafts/<name>/`. `tbot skills submit <dir>` uploads a
  `skill_bundle` artifact.
- **Verify:** a `skill_verify` job (COMP-005) with a per-skill token and compute budget.
  1. Static scan and lint: no symbols, dates or fitted-looking constants; no network,
     credentials or filesystem escape; no `allowed-tools`, hooks or `!` shell; the
     description token count is ≤ 100; the body is ≤ 500 lines.
  2. Unit tests.
  3. **Replay:** the source task plus ≥ 2 variants on synthetic instruments, with vs
     without the skill. It uses a paired sequential design and runs at production model
     and effort. The skill must be activated and change the outcome.
  4. Trigger evals: should, shouldn't and edge cases, via Batch.
  5. Coexistence check against the project's resident set: no trigger stealing.
  6. For `kind=analysis-code`: the PIT truncation test, noise-null test (FPR ≤ 7% at
     5% nominal) and leakage-trap task.
  7. **T5:** tokens or tool calls reduced ≥ 20% at equal success.

  The **Reviewer** sub-agent sees only the bundle, the tests and task descriptions.
- **Admit:** on pass, one transaction writes `skills` (or a new `skill_versions` row
  for a patch), sets `status='provisional'` and scope `project`, and the glossary view
  reflects it. This is the only path to admission, and it is platform-side
  (`skills.admit` is internal). `provisional` becomes `admitted` after N exposures with
  a non-negative helpfulness vote balance.

### 6.4 Promotion, curation, retirement (Set R-b)

- **project → user:** owner approval (`approval_requests.kind='skill_promotion'`).
- **user → global** requires all of:
  - evals on tasks from other projects;
  - a sanitiser (lint plus an LLM check via Batch);
  - owner consent;
  - a security scan;
  - a reviewer who isn't the author (v1: the platform owner);
  - no `tainted` provenance.

  Global skill scripts are limited to `tbot` SDK calls and pure transforms.
- **Curator** (sleep-time sub-agent or offline Batch) reads structured deltas only:
  `findings`, `skill_runs`, verdicts. It proposes playbook bullet deltas and merges of
  look-alike skills, both through the proposal path. Each run has a token cap.
- **Retirement:**
  - archive after N exposures with no activation, or a leave-one-out contribution ≤ 0
    (AGENT-004 skill arms);
  - auto-disable on test failure after a platform, SDK or model bump;
  - deprecated skills keep `alias_of`.
- **Quarantine:** `POST /api/skills/{id}/quarantine` (admin) sets
  `status='quarantined'`, which removes the skill from every glossary view at once and
  triggers a `skill_notice`.

## 7. Instrument profiles and analogs

- `instrument_profile` jobs compute a deterministic profile per
  `(instrument, timeframe)`, on a schedule and when data changes:
  - stylized facts: tails, clustering, Hurst, variance ratios;
  - `data_qc`;
  - the EDGE cost model;
  - coverage;
  - filtered regime history;
  - fitted processes.
- `tbot profile <instr> <tf>` returns a ≤ 400-token summary plus a handle.
- **Analogs** (Set R-b): kNN over the explicit standardised vector (BS-007 12 §7) with a
  time-exclusion zone equal to the window's horizon.

## 8. REST surface

| Route | Scope |
|---|---|
| `POST /api/memory/findings`, `GET /api/memory/findings/search`, `GET /api/memory/findings/{id}` | research:memory |
| `GET /api/profiles/{instrument}/{tf}` | research:data.read |
| `GET /api/skills/glossary`, `GET /api/skills/search`, `GET /api/skills/{id}` | research:skills.propose (read) / web |
| `POST /api/skills/proposals`, `POST /api/skills/proposals/{id}/submit` | research:skills.propose |
| `POST /api/skills/{id}/vote` | research:skills.propose |
| `POST /api/skills/{id}/promote`, `/deprecate`, `/quarantine` | web (owner/admin) |

## 9. Requirements mapping

| BS-007 IDs | Where |
|---|---|
| KM-01…KM-03 | §5 |
| KM-04 | §3 `move_memory` (write path owned by FEAT-004 `strategy.move`) |
| KM-05 | §6.4 (playbook via the proposal path) |
| KM-06, KM-07 | §3–§4 |
| KM-08 | §7 |
| KM-09 | §6.4 curator |
| KM-10 | §5 tainted |
| SK-01, SK-07, SK-09, SK-14 | §6.1–§6.2 |
| SK-02, SK-03 | §3 view + §6.3 admit |
| SK-04…SK-06, SK-08, SK-10, SK-13, SK-15 | §6.3, §3 |
| SK-11, SK-12, SK-16, SK-17 | §6.4 + AGENT-004 |

## 10. Acceptance

1. **(L)** Two users' sessions both materialise the core skills with byte-identical
   resident blocks. The glossary lists every core skill.
2. **(L)** `tbot memory add` with a non-existent evidence id is rejected. A valid
   finding is searchable in the next session (structured filter).
3. **(R-a)** A T2 proposal from a verified trajectory is verified, admitted as a
   provisional project skill, appears in `skill_glossary` in the same transaction, and
   is materialised next session. A speculative proposal without a trajectory is refused.
4. **(R-a)** A skill with a ticker and a fitted constant fails lint. A `bfill` skill
   fails the truncation test.
5. **(R-b)** Quarantining a global skill removes it from all glossaries immediately;
   live sessions receive a notice.
6. **(R-b)** A leave-one-out arm retires a no-contribution skill after the exposure
   threshold.

## 11. Open questions

1. Local embedding model choice (retrieval eval).
2. Default visibility of findings in multi-user deployments.
3. Exposure threshold N for `provisional → admitted` and for retirement.
