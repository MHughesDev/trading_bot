# BS-007 — Quant Research Agent: index, decisions, and how to plan from here

**Brainstorm ID:** BS-007
**Scope:** multi
**Status:** spec-ready draft (restructured 2026-09-11 from the 2026-09-10 brainstorm)
**Author:** Mason Hughes (with Claude)

---

## One-liner

Build the best quant research agent this platform can support. It is a single agent that
runs long jobs in its own container. It answers questions about any instrument, and it
researches, backtests and models trading strategies for stocks and crypto honestly. The
platform is rebuilt around it, and the human UI comes as a byproduct.

## How to use this folder

- **Spec docs (02–16) are normative.** Each one has these sections:
  - scope;
  - locked decisions;
  - design;
  - **requirements with stable IDs** (`RT-01`, `DA-07`, …);
  - interfaces;
  - acceptance criteria;
  - open questions.
  Implementation plans cite requirement IDs.
- **Research docs (`research/`) are informative.** They hold evidence and sources. When a
  research doc and a spec doc differ, the spec wins.
- **Pipeline:**
  1. This folder.
  2. A formal spec in `docs/specs/` plus an ADR in `docs/adr/` (Phase 0).
  3. A plan set in `docs/plans/plan-sets/set-L…`.
  4. The queue.

  [17_BUILD_ORDER.MD](17_BUILD_ORDER.MD) maps every spec doc to its formal spec, ADR and
  plan set.
- **Ground truth:** [01_GROUND_TRUTH.MD](01_GROUND_TRUTH.MD) is the as-built baseline,
  audited 2026-09-10. Gap IDs `G-01…G-14` are used throughout. Re-verify file and line
  references before building.

## Locked decisions

| ID | Decision | Date | Consequence |
|---|---|---|---|
| D-01 | **Frontier models first** (Claude Opus class) | 2026-09-10 | Harness and tool granularity are tuned for frontier models. Local models aren't a constraint on the agent. `crates/llm` stays for `LlmInference` strategy nodes |
| D-02 | **The agent runs in its own sandboxed container**, one per research project, like a coding agent | 2026-09-10 | Data reaches the agent as files; analysis is code; enforcement moves to the platform (03 §2) |
| D-03 | **Local embeddings** | 2026-09-10 | Research notes and text never leave the machine. `apps/embedder` switches from OpenAI (12) |
| D-04 | **One agent, optimised for long-running work.** No modes; asking a question is just a short session | 2026-09-10 | One harness, one tool surface, one UI (02, 03, 15) |
| D-05 | **The platform is agent-first.** The human UI is a viewer and editor over the same APIs | 2026-09-10 | Every capability is an API before it is a page (14, 15) |
| D-06 | **The agent creates skills.** The platform verifies and admits them. Every session for every user gets the core and global skills. Every admitted skill is in the glossary | 2026-09-10 | 13_SKILLS |
| D-07 | **Harness = Claude Agent SDK** (option A) in a container we host | 2026-09-11 | 03_RUNTIME. Managed Agents and the custom loop are no longer in scope. The platform API, SDK and MCP layer stays harness-neutral |
| D-08 | **Two-layer strategies:** Python position-series research strategies, plus Strategy Language v2 for anything deployable | 2026-09-11 | 08_STRATEGIES, 09_BACKTEST |
| D-09 | **Free data sources only** | 2026-09-11 | 06_DATA §6. Existing collectors plus free public sources; paid feeds are out of scope |
| D-10 | **Enforce invariants where the agent can't bypass them:** data cutoff, evaluation service, token scopes, admission services. Prompts and skills only teach | 2026-09-10 | 03 §2 |
| D-11 | **Never trade away model capability for tokens.** Remove waste instead | 2026-09-10 | 04_CONTEXT_AND_COST |
| D-12 | **Research authority only.** No orders, no automations, no live arming. Deployment and promotion are human-approved proposals | standing | 02 §6, 03 §12 |
| D-13 | **Exploration is logged, not counted as trials.** Every Data API read and sandbox job inside a project goes to an exploration ledger, reported with each verdict | 2026-09-11 (was the recommended option) | 11 §3 |

## Document map

| Doc | Subsystem | Req prefix | Proposed formal spec | Plan set | Phase |
|---|---|---|---|---|---|
| [01_GROUND_TRUTH.MD](01_GROUND_TRUTH.MD) | As-built baseline and gap register | G- | — | — | — |
| [02_AGENT.MD](02_AGENT.MD) | What the agent is: jobs, behaviour contract, user interaction, success scorecard | B- | AGENT-001 §1 | L | 1 |
| [03_RUNTIME.MD](03_RUNTIME.MD) | Agent SDK runtime, container, orchestrator, workspace, hooks, sub-agents, protocol, `final_report`, durability, security | RT- | AGENT-001 | L | 1 |
| [04_CONTEXT_AND_COST.MD](04_CONTEXT_AND_COST.MD) | Token efficiency without capability loss: caching, compaction, context budgets, waits, telemetry, hazards | CX- | AGENT-001 §context | L | 1 |
| [05_JOBS_AND_ARTIFACTS.MD](05_JOBS_AND_ARTIFACTS.MD) | Durable job service, artifact store, handles, manifests | JB- | COMP-005 | L | 1 |
| [06_DATA.MD](06_DATA.MD) | Data API v2, PIT and research cutoff, token scopes, `data_qc`, as-of universes, free sources, backfill | DA- | DATA-005 | L (core) / P (sources) | 1, 5 |
| [07_FEATURES.MD](07_FEATURES.MD) | One feature engine (Rust + PyO3), expression features, parity | FE- | DATA-006 | M | 2 |
| [08_STRATEGIES.MD](08_STRATEGIES.MD) | Layer 1 position series and truncation test; Layer 2 Strategy Language v2; reconciliation | ST- | FEAT-004 | M (L1) / O (L2) | 2, 4 |
| [09_BACKTEST.MD](09_BACKTEST.MD) | market_simulator facade, `StrategyInstance`, `RunConfig`, fill timing, batch and vectorised tiers, synthetic instruments | BT- | FEAT-005 | M | 2 |
| [10_MODELS.MD](10_MODELS.MD) | DatasetSpec, model families, bring-your-own models, walk-forward prediction series, scoring and calibration | MD- | FEAT-006 | N | 3 |
| [11_EVALUATION_STANDARD.MD](11_EVALUATION_STANDARD.MD) | Set J adaptations, the research protocol made mechanical, QC rules T1–T20, candidate dossier, family tests | EV- | BACKTEST_SUITE_CORE_SPEC v2 | Q | 2–4 |
| [12_KNOWLEDGE_MEMORY.MD](12_KNOWLEDGE_MEMORY.MD) | Memory layers, findings, move memory, playbook, instrument profiles, local embeddings | KM- | AGENT-003 | R | 6 (core in 1) |
| [13_SKILLS.MD](13_SKILLS.MD) | Agent-created skills: triggers, verification, registry and glossary, materialisation, security, honesty tests | SK- | AGENT-003 §skills | L (core) / R | 1–6 |
| [14_TOOLBOX.MD](14_TOOLBOX.MD) | `tbot` SDK/CLI and MCP surface: principles and full catalogue | TB- | AGENT-002 (supersedes INTG-001 tool list) | spread | 1–6 |
| [15_WORKSPACE_UI.MD](15_WORKSPACE_UI.MD) | Research workspace UI, approvals, event stream | UI- | COMP-006 | L (v1) / later | 1+ |
| [16_AGENT_EVALS.MD](16_AGENT_EVALS.MD) | Agent eval suite, synthetic instruments, graders, non-inferiority, harness evolution | AE- | AGENT-004 | L (skeleton) / all | 1+ |
| [17_BUILD_ORDER.MD](17_BUILD_ORDER.MD) | Phases, plan sets, ADRs, dependencies, exit criteria | — | — | — | — |

### Research (informative)

| Doc | Contents |
|---|---|
| [research/R1_AGENTIC_TECHNIQUES.MD](research/R1_AGENTIC_TECHNIQUES.MD) | Harness, context, search, memory, verification, security and self-improvement techniques, with sources |
| [research/R2_QUANT_TECHNIQUES.MD](research/R2_QUANT_TECHNIQUES.MD) | Quant techniques graded by evidence strength and turned into tools |
| [research/R3_QUANT_KNOWLEDGE_CORE.MD](research/R3_QUANT_KNOWLEDGE_CORE.MD) | Quant knowledge from stochastic calculus to the toolkit, ML, archetypes and playbook. Seed content for the core skills |
| [research/R4_TOKEN_EFFICIENCY_AUDIT.MD](research/R4_TOKEN_EFFICIENCY_AUDIT.MD) | Token audit of the design as if built: per-session estimates, techniques, hazards, sources |

## Conventions

- **MUST / SHOULD / MAY**, as in RFC 2119.
- **Enforcement tags** on rules:
  - **(E)** enforced by a platform service;
  - **(H)** enforced by a harness hook;
  - **(A)** required of the agent and checked by the `final_report` validator or the eval suite.
- **Invariants** carried from Set J and ADR-0023 hold everywhere:
  - **INV-1:** trials are counted automatically and never decrease.
  - **INV-2:** distributions are sealed; nothing picks by argmax.
  - **INV-3:** no significance claim without its null and trial count.
  - **P1:** the LLM proposes structure, and an optimiser chooses the numbers.

## Glossary

| Term | Meaning |
|---|---|
| **Research project** | The unit of work: a persistent workspace container volume, research memory scope, research cutoff and budget, bound to an instrument set or theme. Every conversation belongs to one |
| **Desk project** | Each user's always-present project with cutoff = now. It handles live and recent questions; its Experiments are exploratory only (no G3 or vault) (06 §3.1) |
| **Session** | One run of the agent in a project's container. It can be seconds (a question) or hours (a campaign) |
| **Workspace** | The project's git repo at `/workspace`: NOTEBOOK, plan, code, data, reports (03 §5) |
| **NOTEBOOK.md** | The agent-maintained research state (plan, hypotheses, findings, candidates). Its first section is a ≤2k-token "Current state" |
| **Research cutoff** | A per-project timestamp. The Data API never returns data after it to research tokens; only the vault gate reads past it |
| **Job** | A durable, idempotent unit of slow platform work (backtest, sweep, training, …) (05) |
| **Artifact / handle** | Content-addressed platform output (`art_…`), passed by handle instead of through context |
| **Exploration ledger** | Log of the data looks and sandbox analyses that preceded a hypothesis. Reported, not counted as trials |
| **Effective N** | Trial count adjusted for correlation between trials, feeding DSR |
| **Layer 1 / position series** | A Python research strategy: a pure function from PIT data to target positions, executed by the backtest service |
| **Layer 2 / SLv2** | Strategy Language v2: text → AST → bytecode; the only deployable form |
| **Prediction series** | Walk-forward out-of-fold model predictions, stored as an artifact and read by backtests as a feature stream |
| **Dossier** | The 12-part evidence package a candidate needs before the vault (11 §5) |
| **Skill** | A verified, versioned procedure (SKILL.md plus sidecar) the agent can load (13) |
| **Materialiser** | The orchestrator step that writes a session's skill set and glossary into the container |
| **Glossary** | The registry-generated catalogue of every admitted skill the user can see |

## Cross-cutting open questions

1. **Billing path** for the agent's model calls: an API key through the platform's LLM
   proxy (assumed in 03 §4), or a subscription? This sets cache TTL defaults and budgets.
2. **GPU availability** for training jobs: local GPU, CPU-only, or burst to a cloud
   worker? This affects 10's sequence-model scope.
3. **Multi-tenancy:** will outside users ever share global skills and memory? This
   affects 13 §6 and 12 privacy scope.
4. **Monthly eval budget** for agent evals, skill A/B testing and re-evaluation on model
   upgrades (16 §7).
5. **Desk project** (06 §3.1): confirm the design that separates live questions (Desk,
   cutoff = now, exploratory only) from research projects (fixed cutoff, vault). It was
   introduced during the cleanup to resolve a conflict between J1 live questions and
   holdout integrity.
6. **Paper deployment proposals:** may the agent create a pending paper-deployment request
   for human approval, once live evaluation (G-11) is wired? The assumed answer is yes,
   proposal only.

## Next step

**Phase 0** (17 §1). Write the formal specs and ADRs listed in 17, and run the
**runtime spike**:
- Agent SDK in a container, calling the model through the platform LLM proxy;
- `tbot jobs` plus Data API bars with the cutoff enforced;
- the pure-noise eval task, end to end.

## Links

- [FEAT-003 Strategy Research Agent](../../specs/FEAT-003-strategy-research-agent.md)
- [Backtest suite core spec](../../specs/BACKTEST_SUITE_CORE_SPEC.md)
- [INTG-001 MCP server](../../specs/INTG-001-mcp-server.md)
- [DATA-004 strategy definition format](../../specs/DATA-004-strategy-definition-format.md)
- [ADR-0022](../../adr/0022-mcp-thin-client-and-internal-agent.md)
- [ADR-0023](../../adr/0023-llm-proposes-optimiser-chooses-set-j-judges.md)
- [Set J](../../plans/plan-sets/set-J/MASTER.md)
- [Set K](../../plans/plan-sets/set-K/MASTER.md)
