# BS-007 / 12 — Skills: the agent writes its own, and every skill joins the glossary

Status: draft proposal, 2026-09-10. Based on a research pass over about 40 sources
(sources at the end). Evidence tags: **[Bench]** measured on a benchmark, **[Paper]**
claimed in a paper without a strong benchmark, **[Prod]** production documentation or
practice.

> **Your two requirements, and how this doc meets them.**
> 1. *Skills are used no matter the user.* Every session, for every user, starts by
>    **materialising the skill set**: core skills, then global, then user, then project
>    (§5). This happens in the harness, not the prompt, so no user or session can opt out
>    of core or global skills.
> 2. *New skills always go into the glossary.* A skill can exist in only two places:
>    drafts in the workspace (not usable), or the **registry**. Admission to the registry
>    writes the glossary entry **in the same transaction** (§4). An admitted skill that is
>    missing from the glossary can't exist, and `GLOSSARY.md` is regenerated from the
>    registry at every session start.

---

## 1. What the evidence says, in five lines

| Finding | Implication for us |
|---|---|
| Skills the agent writes *before* attempting a task (speculative) score **8–11.5 pts below no skills at all** (SkillsBench, 2602.12670) [Bench] | Never let the agent write a skill from imagination. The trigger is verified experience. |
| Skills distilled from real attempts and checked by an outside verifier **beat human-written skills** (EvoSkills 71.1% vs 53.5%; SkillEvolver 56.8% vs 43.6%; MUSE 85.2% vs 81.2%) [Bench] | Self-created skills are worth building, but only with an isolated verifier. |
| Learning from your own feedback alone causes "recursive drift" (SkillLearnBench) [Bench] | The verifier must not be the author, and must use outside signals (tests, platform jobs). |
| Selection accuracy collapses once a library passes about 84–92 skills. Look-alike skills cause up to 68% of the loss (2601.04748, 2605.24050) [Bench] | Keep about 40 skills in context. Find the rest by search. Force patches over near-duplicates. |
| About 26% of marketplace skills had a vulnerability; SKILL-INJECT reached up to 80% attack success on frontier models (2601.10338, 2602.20156) [Bench] | A shared cross-user library needs provenance, scanning, taint tracking and human review before anything goes global. |

Supporting numbers:
- **Tiered loading.** HASTE's tiered skills scored 100% medals on 8 competitions. Loading
  all 159 flat scored 62.5%, the same as loading none, at twice the tokens (2606.30911)
  [Paper].
- **Compact skills.** Compact skills gained +19.0 pts where "comprehensive" ones gained
  +0.7 (SkillsBench).
- **Skills as programs.** ASI's program skills beat text skills by +11.3% and used 10–15%
  fewer steps (2504.06821) [Bench].
- **Failure feedback.** Letta found that adding failure feedback took the gain from +21%
  to +37% relative (Letta blog) [Prod].

---

## 2. Where skills sit among the other kinds of memory

Skills are **procedures**. They're different from facts, findings and move statistics,
and mixing them is how look-ahead and alpha leak into a shared library.

| Store | Holds | Example | Written by | Scope |
|---|---|---|---|---|
| **Skills** (this doc) | How to do something | "Build a leak-free triple-barrier label set with the `tbot` SDK", "Diagnose a CPCV path distribution that's bimodal" | Agent drafts → platform verifies | core / global / user / project |
| **Playbook** (09 §3.2 ACE) | Short strategic bullets with helpful/harmful counters | "Before tuning thresholds, check turnover × cost against gross edge" | Curator from session reflection | global / user |
| **Research memory L2** (05 §4) | Findings with Set J citations | "Funding carry on BTC perps: G2 pass, G3 fail (DSR 0.81), exp_…" | Agent via `tbot memory add` | user / project (global only via 11's dossier) |
| **Move memory** (09 §3.1 AlphaMemo) | Which search moves worked in which context | "Adding a vol filter to momentum on 1h crypto improved median Sharpe in 7/12 attempts" | Harness, automatically | user / project |
| **Reference cards** (06, 11) | Quant concepts and QC rules | "Deflated Sharpe ratio: when and how" | Us (curated) | core (shipped as core skills) |

**Rule:** the playbook lives in the skill registry as `kind: playbook` (one versioned
skill per category), so it shares the glossary, versioning and review path. Move memory
and L2 findings stay separate, because they carry data-derived claims.

---

## 3. When the agent creates a skill

### 3.1 Triggers (in the system prompt, and nudged by a harness hook)

| # | Trigger | How it's detected | Evidence |
|---|---|---|---|
| **T1 Repetition** | The same procedure ran ≥3 times in a project, or in ≥2 projects | Harness compares workspace scripts by AST/hash similarity and intent labels in git history. A hook nudges: "this looks like the 3rd time: propose a skill?" | EvoDS τ=3 with 69% reuse; Trace2Skill ≥2 [Bench] |
| **T2 Hard-won and verified** | A subtask took ≥15 tool calls or ≥1 error-then-fix, **and** ended in a platform-verified outcome (job succeeded, tests passed, `final_report` validator passed) | Stop-hook reads the session's tool log and job results | Voyager, AWM, ASI gate on success [Bench]; Hermes uses 5+ calls [Prod] |
| **T3 Pitfall** | A failure with a general root cause: a data quirk, an SDK gotcha, a leak the platform caught | Platform rejections (cutoff breach, truncation-test fail, validator fail) are tagged | Letta, ReasoningBank, HASTE [Bench/Paper] |
| **T4 Human** | An explicit request, or a correction ("always do X") | Steering message classifier | Anthropic's Claude A/B loop [Prod] |
| **T5 Savings** | Replay shows ≥20% fewer tokens or tool calls at equal success | Measured during verification (§4), not claimed | LATM, ASI, HASTE [Bench] |

- **T1–T4 start a proposal.** T5 is a condition for **admission**: a skill that doesn't
  save effort at equal quality isn't admitted, however clean it looks.
- **T3 produces a patch, not a new skill.** It goes into the `## Pitfalls` section of the
  relevant existing skill. Numeric move outcomes go to move memory, not to skills.

### 3.2 Never create a skill when

- the idea is speculative, with no verified trajectory behind it (the SkillsBench result);
- the work is one-off;
- the skill would just wrap one existing platform tool, which is already a tool;
- it contains findings, symbols, venues, dates, tuned parameter values or artifact handles;
- it can't be tested;
- the session's only source was untrusted text (news, social, web pages), per the 09 §6
  quarantine rule.

### 3.3 Patch before create

`tbot skills propose` runs a nearest-neighbour search over the **full body** of every
skill the agent can see.
- Above the similarity threshold, the proposal is converted to a **patch** of the
  existing skill: a version bump plus a diff, reviewed like any change.
- This is the main defence against the look-alike "shadowing" that drives most selection
  loss (2605.24050) [Bench].

---

## 4. Lifecycle and gates

```
propose ─► draft (workspace) ─► verify (platform job + isolated reviewer) ─► admit (registry + glossary, atomic)
                                                                             │
             retire ◄── merge ◄── measure (skill bill of materials per run) ◄┴─► promote (project → user → global)
```

| Stage | Who | What happens | Gate |
|---|---|---|---|
| **Propose** | Agent (or a hook nudge) | `tbot skills propose --reason T1..T4 --from <trajectory>` | Near-duplicate check (§3.3) |
| **Draft** | Agent | Writes `/workspace/skill-drafts/<name>/`: `SKILL.md`, `skill.yaml`, `scripts/`, `tests/`, `evals/` | Must cite a verified trajectory |
| **Verify** | Platform job `skill_verify` plus an **isolated Reviewer sub-agent** that sees only the skill, its tests and task descriptions, never the author's transcript | (1) static scan; (2) unit tests; (3) **ASI-style replay**: the source task and ≥2 variants, with vs without the skill, on synthetic instruments; the skill must be *invoked* and change the outcome; (4) trigger evals (should-trigger, shouldn't-trigger, edge cases); (5) coexistence check against the materialised set (no trigger stealing); (6) quant admission tests (§7); (7) T5 savings check | All pass, no regression |
| **Admit** | Registry | Assigns id, semver, content hash and signature; status `provisional`; **writes the glossary row in the same transaction** | Automatic for project scope |
| **Use** | Agent | Only admitted skills are materialised into `.claude/skills/` (read-only mount) | PreToolUse hook denies writes to `.claude/skills/**` |
| **Measure** | Registry | Per run, a **skill bill of materials**: exposed / activated / excluded, script exit codes, the outcome of the task it was used in, a helpful/harmful vote at session end | — |
| **Promote** | Curator job, then a human | project → user (owner approves) → global (§6) | Evals on other projects' tasks, sanitisation, review |
| **Merge / retire** | Curator job, run as sleep-time work (09 §3.4) during `jobs wait` or overnight | Merge look-alikes if the merged skill's evals match; archive after N exposures with no activation, or a leave-one-out contribution ≤ 0; auto-disable on test failure after a platform, SDK or model version bump | Deprecated skills keep an alias to their replacement, so old transcripts and citations still resolve |

**Why the platform admits and the agent doesn't:** it's the same rule as 08 §1. The agent
runs code in a sandbox, so anything it can write, it can bypass. Admission is a
platform-side write the agent's token can't perform. The agent can only propose.

---

## 5. Materialisation: how skills reach every session, for every user

At session start, the session orchestrator (08 §2) runs the **materialiser**:

1. **Resolve scopes** in precedence order: core > global > user > project. A lower scope
   can't reuse a core or global name, which prevents shadowing.
2. **Always include:**
   - every **core** skill (shipped with the platform: the 06/11 reference cards as skills,
     the `tbot` SDK how-tos, `skill-finder`, `skill-authoring`);
   - every **pinned** global skill.
   This is what "used no matter the user" means mechanically.
3. **Fill to about 40 descriptions** in `.claude/skills/` with the highest-helpfulness
   skills for this project's asset class and theme. Forty is below the ~50-skill level
   where shadowing starts and well below the ~84–92 collapse point.
4. **Write `GLOSSARY.md`** into the `skill-finder` core skill, generated from the registry
   and filtered to what this user can see:
   - a category tree (`data/`, `features/`, `labels/`, `validation/`, `models/`,
     `strategies/`, `execution-costs/`, `reporting/`, `platform/`, `playbook/`);
   - one line per skill: name, one-line purpose, scope, status, version.
5. **Put the rest in `skills-library/`** (not auto-loaded), reachable by
   `tbot skills search "<need>"`:
   - hybrid BM25 plus the local embedding model (05 §7) over the **full skill body**, not
     only the description (dropping the body costs 29–44 pts, SkillRouter 2603.22455)
     [Bench];
   - an optional rerank.
6. **Verify** hashes and signatures in the container. A registry revocation or quarantine
   propagates to live sessions at their next turn.

**Two-stage routing:** the agent sees about 40 descriptions plus the glossary's category
tree. For anything else it reads a category, then searches. Two-stage hierarchical routing
recovers 9–40 points over flat selection at scale (2601.04748) [Bench].

**Harness mechanics:**
- **Option A (Agent SDK):** the materialiser writes `.claude/skills/` itself before
  starting the SDK. Skills load live.
- **Option B (Managed Agents):** upload through the Skills API with version pins, up to
  500 per session. Self-hosted sandboxes can't use repository skill discovery, so the
  materialiser drives it through the API.
- **Option C:** a custom loop implements progressive disclosure itself. Descriptions go in
  the system prompt; bodies load on demand via a `load_skill` tool.

---

## 6. Scope, promotion, privacy and security

| Scope | Visible to | How a skill gets here |
|---|---|---|
| **core** | Everyone, always materialised | Shipped by us with the platform; changed only through a repo PR |
| **global** | Every user | Promotion from user scope (gates below) |
| **user** | One user's projects | Owner approves a verified project skill |
| **project** | One project | Automatic after verification |

**Promotion to global requires all of the following:**

1. **Evals pass on tasks from other projects** (not the source project).
2. **A HASTE-style sanitisation pass** removes symbols, venues, dates, parameter values,
   artifact and strategy handles, user names and finding ids. It runs as a lint plus an
   LLM checker. Skills referencing private artifacts fail automatically.
3. **The owner consents** to sharing.
4. **A static security scan** checks network use, credential access, filesystem scope and
   instruction manipulation.
5. **A human reviewer who isn't the author approves.** For v1 that's you.
6. **No untrusted taint:** the skill's provenance shows no untrusted inputs.

**Security rules for every scope:**
- **No capability grants.** Agent-authored skills may not use `allowed-tools`, `hooks` or
  `!` shell injection. Skills describe how to do things; they never grant permission to
  do them.
- **Scripts run in the sandbox** with the research-scope token and no egress beyond the
  platform API.
- **A quarantine flag** disables a skill everywhere immediately.
- **Signed commits and pinned versions**, with the last version kept for rollback (per
  Anthropic's enterprise guidance).
- **A shared library is shared memory with code-execution reach.** Queries alone could
  poison it (MINJA reached 98% injection, 2503.03704) [Bench]. That's why global requires
  review, not only automated gates.

---

## 7. Honest-evaluation rules for skills (quant-specific)

**Skills hold procedures, never findings.** On top of the invariants in 11 §1:

| Rule | Enforcement |
|---|---|
| No market claims or tuned values in a skill | Lint (symbols, numbers that look like fitted parameters, dates) plus reviewer; failures reject the skill |
| Skills never compute performance claims | Skill outputs are labelled `exploratory`. The `final_report` validator already rejects performance claims without a Set J artifact. |
| No client-side search-until-significant loops | Lint forbids submit-backtest-in-a-loop plus argmax patterns; use `tbot sweep`. Every submission is still counted by the evaluation service (INV-1), whatever a skill does. |
| Skills never read the holdout | Data API cutoff (08 §3.1) enforces this regardless of skill code |
| **PIT truncation test** for every `analysis-code` skill | Outputs up to time *t* must be unchanged when data after *t* is removed. Catches full-sample normalisation, `bfill`, centred windows. |
| **Noise-null test** | On synthetic pure-noise instruments, the skill's "signal found" rate must be ≤ the nominal rate (e.g. FPR ≤ 7% at a 5% nominal level) |
| **Leakage-trap task** | The skill must not fall into the planted leak |
| **No selecting skills on real market data** | Replay and A/B evaluation for skills runs on synthetic instruments only. Choosing skills by their backtest wins would be multiple testing at the level of the library. |

Risks I inferred for our setup (no direct study):
- a skill that makes a look-ahead bug permanent;
- a skill that encodes a hidden search loop;
- a skill that smuggles a fitted parameter, which is both an alpha leak and a claim that
  skips Set J.

The rules above target each of these directly.

---

## 8. File format

`SKILL.md` stays compliant with the open Agent Skills spec, so it works in the Agent SDK,
in Managed Agents and in any other runtime:

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

Keep it under 500 lines. Put longer detail in `reference/` files the skill links to
(progressive disclosure).

Everything the open spec can't hold goes in a sidecar `skill.yaml`, which the registry
owns:

```yaml
id: skl_01J...           version: 2.1.0        content_sha256: ...
scope: project|user|global|core
status: draft|provisional|admitted|deprecated|quarantined
kind: procedure|analysis-code|platform-howto|pitfalls|playbook
category: labels/event-based                     # glossary path
provenance:
  author: agent|user:<id>|core
  session: ses_...   model: claude-opus-5   harness: 1.4.2
  source_trajectory: art_...   parents: [skl_...@2.0.0]
  trigger: T2        untrusted_inputs_seen: false
requires:
  platform_api: ">=2.3"   tbot_sdk: ">=0.9"
  tools: [data.bars, jobs.submit]   python: [polars>=1.0]
  data_scope: pre-cutoff
honesty:
  computes_performance_claims: false   reads_holdout: false
  submits_trials: via-jobs-api          output_label: exploratory
tests:
  unit: tests/   replay: evals/replay.json   trigger: evals/trigger.json
  pit_truncation: true   noise_null: {fpr_max: 0.07}   leakage_trap: true
review: {reviewer: ..., approved_at: ..., signature: ...}
```

**Usage statistics live only in the registry, never in the file.** That stops the agent
editing its own counters, and keeps skill files byte-stable so prompt caching isn't
disturbed.

---

## 9. Registry (the glossary's source of truth)

- **Postgres tables:**
  - `skills` (id, name, scope, category, status, current_version);
  - `skill_versions` (bundle hash, spec, provenance, review);
  - `skill_runs` (the bill of materials per session: exposed, activated, outcome);
  - `skill_evals` (replay, trigger and quant-test results);
  - `skill_votes`.
- **Bundles** are content-addressed in the artifact store.
- **Glossary** = a view over `skills` joined to the current admitted version, rendered as
  a category tree. Every admission, patch, deprecation or quarantine changes that view
  inside the same transaction, which is how "always in the glossary" is guaranteed.
- **Interfaces:** `tbot skills search | show | glossary | propose | patch | vote |
  deprecate`, plus matching MCP tools (group `skills.*`) for option B/C harnesses and the
  UI.
- **UI:** a Skills page in the workspace showing the glossary tree, per-skill version
  history, bill-of-materials stats, eval results, and the promotion queue with approve
  and reject buttons for you.

---

## 10. Measuring whether skills help

This runs in the agent eval suite (04 §9: planted-edge, pure-noise, leakage-trap and Q&A
tasks on synthetic instruments).

- **Arms:**
  - (a) core skills only;
  - (b) the full materialised library;
  - (c) leave-one-out for the 10 most-used skills, rotated monthly.
- **Design:**
  - paired tasks;
  - ≥3 seeds (HASTE's single-seed result is a warning);
  - fresh synthetic seeds each run;
  - 30–40% of tasks held out, never used for admission or description tuning.
- **Metrics:**
  - success, and false-discovery rate on noise;
  - leakage caught;
  - trials used per correct verdict;
  - tokens, wall time and tool calls;
  - honesty violations;
  - trigger precision and recall.
- **Actions:** a skill whose leave-one-out result is ≤ 0 after enough exposure is retired
  (SLIM +7.1 pts from pruning, 2605.10923) [Bench]. The whole library is re-evaluated on
  every model upgrade (open question 5).

---

## 11. Build placement (08 §10)

| Phase | Skills work |
|---|---|
| **1 Agent foundation** | Core skills shipped as files (06/11 cards, `tbot` how-tos, `skill-finder` with a static glossary). Read-only `.claude/skills/` mount. PreToolUse deny-write hook. |
| **2–3** | Registry tables and glossary view; materialiser; `tbot skills search/show/glossary`; skill bill of materials logging |
| **4** | `propose/patch` plus the `skill_verify` job (static scan, unit tests, replay on synthetic instruments, quant tests); isolated Reviewer sub-agent; project-scope admission; T1/T2/T3 hooks |
| **6** | Promotion flow and UI; sanitiser; curator (merge/retire) as sleep-time work; skills A/B arms in the eval suite; ACE playbook as `kind: playbook` |

---

## 12. Open questions (defaults in bold)

1. **Who approves promotion to global:** **only you for v1**, or an automated gate once
   evals mature?
2. **Multi-tenancy:** if every user is you or a trusted collaborator, the sanitiser can be
   lighter. **Assume outside users eventually:** global stays opt-in, with strict lint.
3. **Scripts at global scope:** **allowed only as calls into the `tbot` SDK plus pure data
   transforms**, with no network or filesystem access outside the workspace. The
   alternative is instructions only (safer, less useful).
4. **Replay budget:** T5 replays and A/B arms dominate lifecycle cost. **Cap per-skill
   verification at a fixed compute and token budget,** charged to a platform budget, not
   the user's project.
5. **Re-evaluate on model upgrades:** **yes**, the whole library, before the new model
   becomes the default.

---

## Sources

- **Self-creation and verification:**
  - SkillsBench [2602.12670](https://arxiv.org/html/2602.12670)
  - EvoSkills [2604.01687](https://arxiv.org/html/2604.01687v1)
  - SkillEvolver [2605.10500](https://arxiv.org/abs/2605.10500)
  - Trace2Skill [2603.25158](https://arxiv.org/pdf/2603.25158)
  - MUSE-Autoskill [2605.27366](https://arxiv.org/pdf/2605.27366)
  - SkillLearnBench [2604.20087](https://arxiv.org/abs/2604.20087)
  - Voyager [2305.16291](https://arxiv.org/html/2305.16291)
  - AWM [2409.07429](https://arxiv.org/abs/2409.07429)
  - ASI [2504.06821](https://arxiv.org/html/2504.06821)
  - SkillWeaver [2504.07079](https://arxiv.org/abs/2504.07079)
  - LATM [2305.17126](https://arxiv.org/abs/2305.17126)
  - CREATOR [2305.14318](https://arxiv.org/abs/2305.14318)
  - CRAFT [2309.17428](https://arxiv.org/abs/2309.17428)
  - TroVE [2401.12869](https://arxiv.org/abs/2401.12869)
  - HASTE [2606.30911](https://arxiv.org/html/2606.30911)
  - Letta skill learning ([blog](https://www.letta.com/blog/skill-learning/))
- **Triggers and memory:**
  - EvoDS [2606.03841](https://arxiv.org/html/2606.03841v1)
  - ReasoningBank [2509.25140](https://arxiv.org/abs/2509.25140)
  - ExpeL [2308.10144](https://arxiv.org/html/2308.10144)
  - SLIM [2605.10923](https://arxiv.org/abs/2605.10923)
  - Hermes Agent ([docs](https://hermes-agent.nousresearch.com/docs/user-guide/features/skills))
- **Selection at scale:**
  - [2601.04748](https://arxiv.org/html/2601.04748v2)
  - Skill shadowing [2605.24050](https://arxiv.org/html/2605.24050v1)
  - SkillRouter [2603.22455](https://arxiv.org/html/2603.22455v1)
  - SkillRet [2605.05726](https://arxiv.org/abs/2605.05726)
- **Architecture:** Harnessing Agent Skills [2606.20631](https://arxiv.org/html/2606.20631)
- **Security:**
  - Skills in the Wild [2601.10338](https://arxiv.org/abs/2601.10338v1)
  - SKILL-INJECT [2602.20156](https://arxiv.org/abs/2602.20156)
  - SoK: Agentic Skills [2602.20867](https://arxiv.org/pdf/2602.20867)
  - MINJA [2503.03704](https://arxiv.org/abs/2503.03704)
  - MEXTRA [2502.13172](https://arxiv.org/abs/2502.13172)
- **Quant:**
  - FactorMiner [2602.14670](https://arxiv.org/abs/2602.14670)
  - R&D-Agent(Q) [2505.15155](https://arxiv.org/abs/2505.15155)
  - Sanity Checks for Agentic Data Science [2604.11003](https://arxiv.org/abs/2604.11003)
- **Anthropic and the open spec:**
  - [Equipping agents with Agent Skills](https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills)
  - [Skills overview](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/overview)
  - [Best practices](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices)
  - [Enterprise](https://platform.claude.com/docs/en/agents-and-tools/agent-skills/enterprise)
  - [Managed Agents skills](https://platform.claude.com/docs/en/managed-agents/skills)
  - [Claude Code skills](https://code.claude.com/docs/en/skills)
  - [skill-creator](https://github.com/anthropics/skills/blob/main/skills/skill-creator/SKILL.md)
  - [agentskills.io spec](https://agentskills.io/specification)
