# BS-007 / 13 — Skills: the agent writes its own, and every skill joins the glossary

**Spec status:** spec-ready draft · **Req prefix:** SK- · **Formal spec:** AGENT-003
§skills · **ADR:** ADR-0028 (skill registry and admission) · **Plan set:** L (core skills)
/ R (lifecycle) · **Decisions:** D-06, D-10, D-11 · **Evidence tags:** [Bench] measured
on a benchmark · [Paper] claimed without a strong benchmark · [Prod] production docs or
practice

> **The two requirements, and how this spec meets them:**
> 1. **Skills are used no matter the user.** Every session, for every user, starts with
>    the orchestrator **materialising** core, then global, then user, then project skills
>    (§5). This is done in the harness, not the prompt, so no user or session can opt out
>    of core or global skills.
> 2. **New skills always join the glossary.** A skill exists in only two places: drafts
>    in the workspace (not usable), or the **registry**. Admission writes the glossary row
>    **in the same transaction** (§9). An admitted skill missing from the glossary can't
>    exist. The glossary is regenerated from the registry.

---

## 1. Evidence behind the design

| Finding | Design consequence |
|---|---|
| Skills written *before* attempting a task scored **8–11.5 pts below no skills** (SkillsBench, 2602.12670) [Bench] | Triggers require verified experience; speculative skills are banned (§3) |
| Experience-distilled skills checked by an outside verifier **beat human-written ones** (EvoSkills 71.1% vs 53.5%; SkillEvolver 56.8% vs 43.6%; MUSE 85.2% vs 81.2%) [Bench] | Self-created skills are worth building, with an isolated verifier (§4) |
| Self-feedback alone causes "recursive drift" (SkillLearnBench) [Bench] | The verifier is not the author and uses outside signals: tests, platform jobs |
| Selection accuracy collapses past about 84–92 skills; look-alike skills cause up to 68% of the loss (2601.04748, 2605.24050) [Bench] | About 40 resident skills; two-stage routing; patch before create (§3.3, §5) |
| Compact skills gained +19.0 pts, "comprehensive" ones +0.7 (SkillsBench) | Description ≤ 100 tokens; body ≤ 500 lines, with detail in reference files (§8) |
| Tiered loading: 100% medals vs 62.5% flat, the same as none, at 2× tokens (HASTE 2606.30911) [Paper] | Tiered scopes and hierarchical glossary |
| About 26% of marketplace skills had a vulnerability; SKILL-INJECT reached up to 80% attack success (2601.10338, 2602.20156) [Bench] | Provenance, scanning, taint tracking, human review before global (§6) |

## 2. Skills vs other memory

Skills are **procedures**. Facts, findings and move statistics live elsewhere (12).
Mixing them is how look-ahead and alpha leak into a shared library.

| Store | Holds | Example | Written by | Scope |
|---|---|---|---|---|
| **Skills** | How to do something | "Build leak-free triple-barrier labels with the `tbot` SDK" | Agent drafts → platform verifies | core / global / user / project |
| **Playbook** (`kind: playbook`) | Short strategic bullets with counters | "Before tuning thresholds, check turnover × cost against gross edge" | Curator deltas → admission | global / user |
| Findings (12 §3) | Cited claims | "Funding carry on BTC perps: G2 pass, G3 fail" | Agent via `tbot memory add` | project / user |
| Move memory (12 §4) | Move outcomes by context | "Vol filter on 1h crypto momentum improved median in 7/12" | Harness, automatically | project / user |

## 3. When the agent creates a skill

### 3.1 Triggers

| # | Trigger | Detection | Evidence |
|---|---|---|---|
| **T1 Repetition** | The same procedure ran ≥ 3 times in a project, or in ≥ 2 projects | Harness compares workspace scripts by AST/hash similarity and intent labels in git history; the Stop hook nudges | EvoDS τ=3, 69% reuse; Trace2Skill ≥ 2 [Bench] |
| **T2 Hard-won and verified** | ≥ 15 tool calls or ≥ 1 error-then-fix, **and** a platform-verified outcome (job succeeded, tests passed, report validated) | The Stop hook reads the session's tool log and job results | Voyager, AWM, ASI [Bench] |
| **T3 Pitfall** | A failure with a general root cause (data quirk, SDK gotcha, a leak the platform caught) | Platform rejections are tagged (cutoff breach, truncation-test fail, validator fail) | Letta, ReasoningBank, HASTE |
| **T4 Human** | An explicit request, or a correction ("always do X") | Steering-message classifier | Anthropic's Claude A/B loop [Prod] |
| **T5 Savings** | Replay shows ≥ 20% fewer tokens or tool calls at equal success | Measured in verification (§4), not claimed | LATM, ASI, HASTE [Bench] |

- T1–T4 start a proposal. **T5 is an admission condition.**
- T3 produces a **patch** to the relevant skill's `## Pitfalls` section.
- Numeric move outcomes go to move memory, not skills.
- The Stop hook nudges **at most once per session** (04 §10, H18). A draft written while
  the context is still cached is cheapest.

### 3.2 Never create a skill when

- it's speculative;
- it's one-off;
- it only wraps one existing platform tool;
- it contains findings, symbols, venues, dates, tuned values or artifact handles;
- it can't be tested;
- the session's only source was untrusted text.

### 3.3 Patch before create

`tbot skills propose` runs a nearest-neighbour search over the **full body** of every
skill the agent can see. Above the similarity threshold, the proposal becomes a **patch**
(version bump plus diff). This is the main defence against look-alike shadowing.

## 4. Lifecycle and gates

```
propose → draft (workspace) → verify (platform job + isolated reviewer) → admit (registry + glossary, atomic)
                                                                        ↓
          retire ← merge ← measure (skill bill of materials per run) ←──┴──→ promote (project → user → global)
```

| Stage | Who | What happens | Gate |
|---|---|---|---|
| **Propose** | Agent (or hook nudge) | `tbot skills propose --reason T1..T4 --from <trajectory>` | Near-duplicate check (§3.3) |
| **Draft** | Agent | `/workspace/skill-drafts/<name>/`: `SKILL.md`, `skill.yaml`, `scripts/`, `tests/`, `evals/` | Must cite a verified trajectory |
| **Verify** | `skill_verify` job (05) plus an **isolated Reviewer** (03 §9) that sees only the skill, tests and task descriptions | (1) static scan; (2) unit tests; (3) **replay**: source task plus ≥ 2 variants, with vs without the skill, on synthetic instruments, sequential paired design (§10); the skill must be *invoked* and change the outcome; (4) trigger evals via Batch; (5) coexistence check against the materialised set; (6) quant admission tests (§7); (7) T5 savings | All pass; no regression; per-skill verification budget |
| **Admit** | Registry | id, semver, content hash, signature; status `provisional`; **glossary row written in the same transaction** | Automatic for project scope |
| **Use** | Agent | Only admitted skills are materialised into read-only `.claude/skills/` | PreToolUse denies writes to `.claude/**` |
| **Measure** | Registry | Per-run **skill bill of materials**: exposed, activated or excluded; script exit codes; task outcome; helpful/harmful vote at session end | — |
| **Promote** | Curator job, then a human | project → user (owner approves) → global (§6) | Evals on other projects' tasks; sanitisation; review |
| **Merge / retire** | Curator (sleep-time or Batch) | Merge look-alikes if the merged skill's evals match; archive after N exposures without activation, or when leave-one-out contribution ≤ 0; auto-disable on test failure after a platform, SDK or model version bump | Deprecated skills keep an alias |

**Why the platform admits and the agent doesn't:** the enforcement principle (03 §2).
Admission is a platform-side write that the agent's token can't perform
(`skills.admit` is never granted).

## 5. Materialisation: how skills reach every session, for every user

At session start, the orchestrator's materialiser:
1. **Resolves scopes** in precedence order core > global > user > project. Lower scopes
   can't reuse core or global names.
2. **Always includes** every **core** skill and every **pinned** global skill:
   - 11 §7 QC rules and research/R3 reference content as skills;
   - `tbot` how-tos;
   - `skill-finder`;
   - `skill-authoring`;
   - the playbook.
   This is the mechanical meaning of "used no matter the user".
3. **Fills to about 40 descriptions** with the project's selected skills. **The selection
   is frozen per project version.** It is recomputed from helpfulness stats only when the
   project version is bumped, between sessions, never per session. Order is deterministic
   (tier, then name), and descriptions contain no counts, dates or rankings. This keeps
   the prefix byte-stable (04 §3, H3). A shared core block comes first so it caches across
   users.
4. **The glossary** (`GLOSSARY.md`: a category tree, one line per skill with name,
   purpose, scope, status and version, filtered to what the user can see) lives **inside
   the `skill-finder` skill**. It loads on demand and is not resident.
5. **The library** (everything else) is reachable with `tbot skills search "<need>"`:
   hybrid BM25 plus local embeddings over the **full skill body** (12 §6), with a rerank.
6. **Verifies** hashes and signatures. A revocation or quarantine reaches live sessions as
   an **appended system message**, plus removal at the next session start. The system
   prompt is never edited mid-session.

**Two-stage routing:** about 40 resident descriptions, then a glossary category, then
search.

## 6. Scope, promotion, privacy and security

| Scope | Visible to | Enters by |
|---|---|---|
| **core** | Everyone, always materialised | Repo PR (shipped with the platform) |
| **global** | Every user | Promotion from user scope |
| **user** | One user's projects | Owner approves a verified project skill |
| **project** | One project | Automatic after verification |

**Promotion to global requires all of:**
1. evals passing on tasks from *other* projects;
2. a **sanitisation pass** (lint plus LLM check via Batch) removing symbols, venues,
   dates, parameter values, artifact and strategy handles, user names and finding ids;
3. owner consent;
4. a static security scan (network, credentials, filesystem scope, instruction
   manipulation);
5. a human reviewer who isn't the author (**v1: you**);
6. no `tainted` provenance.

**Security rules at every scope:**
- **No capability grants.** Agent-authored skills may not use `allowed-tools`, `hooks` or
  `!` shell injection.
- **Scripts in global skills** are limited to `tbot` SDK calls plus pure data transforms.
- Scripts run in the sandbox with the research scope and no extra egress.
- A quarantine flag disables a skill everywhere at once.
- Signed bundles and pinned versions, with the last version kept for rollback.

## 7. Honest-evaluation rules for skills (quant-specific)

**Skills hold procedures, never findings.**

| Rule | Enforcement |
|---|---|
| No market claims or tuned values | Lint (symbols, fitted-looking numbers, dates) plus Reviewer |
| Skills never compute performance claims | Outputs labelled `exploratory`; the validator rejects claims without a Set J artifact |
| No client-side search-until-significant loops | Lint forbids submit-in-loop plus argmax; use `tbot exp sweep`. Every submission is counted anyway (INV-1) |
| Never read the holdout | Data API cutoff (06 §3), regardless of skill code |
| **PIT truncation test** for `analysis-code` skills | Outputs up to *t* unchanged when data after *t* is removed |
| **Noise-null test** | On pure-noise synthetic instruments, "signal found" rate ≤ nominal (FPR ≤ 7% at 5%) |
| **Leakage-trap task** | The skill must not fall into a planted leak |
| No selecting skills on real market data | Replay and A/B evaluation use synthetic instruments only (avoids library-level multiple testing) |

## 8. File format

`SKILL.md` follows the open Agent Skills spec:

```markdown
---
name: triple-barrier-labels
description: Use when building event-based labels (triple-barrier / meta-labeling) for a
  classifier from bars. Covers vol-scaled barriers, overlapping-label weights, and the
  purge/embargo handoff to CPCV.
metadata:
  tbot-id: skl_01J...
  tbot-version: 2.1.0
---
## When to use
## Procedure        (calls into the tbot SDK; scripts/ for deterministic steps)
## Pitfalls         (patched by T3)
## Verify           (how the agent checks its own output)
```

- Description ≤ 100 tokens, enforced with token counting at admission.
- Body ≤ 500 lines, with longer detail in linked `reference/` files.

**Sidecar `skill.yaml`** (owned by the registry):

```yaml
id: skl_01J...   version: 2.1.0   content_sha256: ...
scope: project|user|global|core     status: draft|provisional|admitted|deprecated|quarantined
kind: procedure|analysis-code|platform-howto|pitfalls|playbook
category: labels/event-based                       # glossary path
provenance: { author: agent|user:<id>|core, session: ses_..., model: ..., harness: ...,
              source_trajectory: art_..., parents: [skl_...@2.0.0], trigger: T2,
              untrusted_inputs_seen: false }
requires:   { platform_api: ">=2.3", tbot_sdk: ">=0.9", tools: [data.bars, jobs.submit],
              python: [polars>=1.0], data_scope: pre-cutoff }
honesty:    { computes_performance_claims: false, reads_holdout: false,
              submits_trials: via-jobs-api, output_label: exploratory }
tests:      { unit: tests/, replay: evals/replay.json, trigger: evals/trigger.json,
              pit_truncation: true, noise_null: {fpr_max: 0.07}, leakage_trap: true }
review:     { reviewer: ..., approved_at: ..., signature: ... }
```

**Usage statistics live only in the registry, never in the file.** That stops the agent
editing its own counters, and keeps the files byte-stable for caching.

## 9. Registry (the glossary's source of truth)

- **Postgres tables:**
  - `skills` (id, name, scope, category, status, current_version);
  - `skill_versions` (bundle hash, spec, provenance, review);
  - `skill_runs` (bill of materials);
  - `skill_evals`;
  - `skill_votes`.
- **Bundles** are content-addressed artifacts (05 §6).
- **The glossary** is a view over `skills` joined to each skill's current admitted
  version. Admission, patch, deprecation and quarantine all change it **in the same
  transaction**.
- **CLI** `tbot skills search | show | glossary | propose | patch | vote | deprecate`, plus
  matching MCP tools for the UI and external clients.
- **UI:** a Skills page with the glossary tree, version history, bill-of-materials stats,
  eval results and the promotion queue (15).

## 10. Measuring whether skills help

This runs in the agent eval suite (16), on synthetic instruments.

- **Arms:**
  - (a) core skills only;
  - (b) the full materialised set;
  - (c) leave-one-out for the 10 most-exposed skills, rotated monthly, and **only for
    skills with enough exposure**.
- **Design:**
  - paired tasks, with common random seeds across arms;
  - **sequential stopping with alpha spending** (stop when the CI excludes 0 or reaches
    the futility bound);
  - ≥ 3 seeds;
  - 30–40% of tasks held out, never used for admission or description tuning;
  - production model and effort.
  This is an estimated 3–10× cheaper at the same error rates (04 §8).
- **Metrics:**
  - success;
  - FDR on noise;
  - leakage caught;
  - trials per correct verdict;
  - tokens, wall time and tool calls;
  - honesty violations;
  - trigger precision and recall.
- **Actions:**
  - retire a skill whose leave-one-out result is ≤ 0 after enough exposure (SLIM
    +7.1 pts);
  - re-evaluate the library before a new model becomes the default.

## 11. Requirements

| ID | Requirement | Pri |
|---|---|---|
| SK-01 | Every session MUST materialise core and pinned global skills regardless of user (§5.2) | MUST |
| SK-02 | Admission MUST write the glossary row in the same transaction; no admitted skill MAY be absent from the glossary | MUST |
| SK-03 | The agent MUST only propose; admission MUST be a platform-side write, and `skills.admit` MUST never be granted to agent tokens | MUST |
| SK-04 | Proposals MUST cite a verified trajectory and pass the §3.3 near-duplicate check (patch above threshold) | MUST |
| SK-05 | `skill_verify` MUST run the §4 checks with an isolated Reviewer, including replay on synthetic instruments and the T5 savings condition | MUST |
| SK-06 | `analysis-code` skills MUST pass the PIT truncation, noise-null and leakage-trap tests | MUST |
| SK-07 | The materialised set MUST be ≤ 40 descriptions, deterministic in order, frozen per project version, with no dynamic content | MUST |
| SK-08 | Descriptions MUST be ≤ 100 tokens (token-counted at admission); bodies ≤ 500 lines | MUST |
| SK-09 | The glossary MUST be generated from the registry and served inside `skill-finder` on demand | MUST |
| SK-10 | `tbot skills search` MUST index full skill bodies with hybrid retrieval | MUST |
| SK-11 | Promotion to global MUST meet all six §6 conditions | MUST |
| SK-12 | Agent-authored skills MUST NOT grant capabilities; global scripts are limited to SDK calls plus pure transforms | MUST |
| SK-13 | A skill bill of materials MUST be recorded per run; usage stats MUST live only in the registry | MUST |
| SK-14 | Revocations MUST reach live sessions without editing the system prompt | MUST |
| SK-15 | The Stop hook MUST nudge at most once per session | MUST |
| SK-16 | Skill A/B evaluation MUST follow §10 (paired, sequential, held-out, synthetic, production model and effort) | MUST |
| SK-17 | The library MUST be re-evaluated before a model upgrade becomes the default | MUST |

## 12. Acceptance criteria

1. A skill proposed from a verified trajectory is verified, admitted, appears in the
   glossary in the same transaction, and is materialised in the next session. A
   speculative proposal with no trajectory is refused.
2. A skill containing a ticker and a fitted threshold fails lint. A skill with a `bfill`
   leak fails the truncation test.
3. Two sessions for different users both receive the core and pinned global skills, with
   byte-identical shared prefix blocks.
4. Quarantining a skill removes it from every glossary view immediately, and live
   sessions receive an appended notice.
5. A leave-one-out arm retires a skill with no measurable contribution after the exposure
   threshold.

## 13. Open questions (defaults in bold)

1. Global promotion approver: **you only for v1**; automate once evals mature.
2. Multi-tenancy: **assume outside users eventually**, so global is opt-in with strict
   lint.
3. Verification budget: **a fixed per-skill compute and token cap, charged to a platform
   budget**, not the user's project.

## Sources

- **Self-creation and verification:**
  - [SkillsBench](https://arxiv.org/html/2602.12670)
  - [EvoSkills](https://arxiv.org/html/2604.01687v1)
  - [SkillEvolver](https://arxiv.org/abs/2605.10500)
  - [Trace2Skill](https://arxiv.org/pdf/2603.25158)
  - [MUSE-Autoskill](https://arxiv.org/pdf/2605.27366)
  - [SkillLearnBench](https://arxiv.org/abs/2604.20087)
  - [Voyager](https://arxiv.org/html/2305.16291)
  - [AWM](https://arxiv.org/abs/2409.07429)
  - [ASI](https://arxiv.org/html/2504.06821)
  - [SkillWeaver](https://arxiv.org/abs/2504.07079)
  - [LATM](https://arxiv.org/abs/2305.17126)
  - [CREATOR](https://arxiv.org/abs/2305.14318)
  - [CRAFT](https://arxiv.org/abs/2309.17428)
  - [TroVE](https://arxiv.org/abs/2401.12869)
  - [HASTE](https://arxiv.org/html/2606.30911)
  - [Letta skill learning](https://www.letta.com/blog/skill-learning/)
- **Triggers and memory:**
  - [EvoDS](https://arxiv.org/html/2606.03841v1)
  - [ReasoningBank](https://arxiv.org/abs/2509.25140)
  - [ExpeL](https://arxiv.org/html/2308.10144)
  - [SLIM](https://arxiv.org/abs/2605.10923)
  - [Hermes Agent](https://hermes-agent.nousresearch.com/docs/user-guide/features/skills)
- **Selection:**
  - [2601.04748](https://arxiv.org/html/2601.04748v2)
  - [Skill shadowing](https://arxiv.org/html/2605.24050v1)
  - [SkillRouter](https://arxiv.org/html/2603.22455v1)
  - [SkillRet](https://arxiv.org/abs/2605.05726)
  - [Harnessing Agent Skills](https://arxiv.org/html/2606.20631)
- **Security:**
  - [Skills in the Wild](https://arxiv.org/abs/2601.10338v1)
  - [SKILL-INJECT](https://arxiv.org/abs/2602.20156)
  - [SoK](https://arxiv.org/pdf/2602.20867)
  - [MINJA](https://arxiv.org/abs/2503.03704)
  - [MEXTRA](https://arxiv.org/abs/2502.13172)
- **Quant:**
  - [FactorMiner](https://arxiv.org/abs/2602.14670)
  - [R&D-Agent(Q)](https://arxiv.org/abs/2505.15155)
  - [Sanity checks for agentic data science](https://arxiv.org/abs/2604.11003)
- **Anthropic and the open spec:**
  - [Agent Skills](https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills)
  - [overview](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/overview)
  - [best practices](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices)
  - [enterprise](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/enterprise)
  - [Claude Code skills](https://code.claude.com/docs/en/skills)
  - [skill-creator](https://github.com/anthropics/skills/blob/main/skills/skill-creator/SKILL.md)
  - [agentskills.io spec](https://agentskills.io/specification)
