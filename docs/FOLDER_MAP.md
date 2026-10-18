# Docs Folder Map

**Last Updated:** 2026-10-03 (Set G task G-4.1, rebuilt from the tree on disk)

> Start at [`README.md`](README.md) for navigation. For **what to work on**, read
> [`plans/PLAN.md`](plans/PLAN.md) first, then [`plans/ROADMAP.md`](plans/ROADMAP.md).
> This file only maps where things live. Counts below are file counts on 2026-10-03 and
> will drift.
>
> **Case note:** on disk the spec folder is `docs/Specs/`, but git tracks it as `docs/specs/`.
> Links in this repo use `specs/`, which resolves on the case-insensitive Windows checkout.
> Git also tracks **both** `docs/specs/README.md` and `docs/specs/README.MD`, and only one
> exists on a Windows checkout (see Set G MASTER, remainder item 4).

```
docs/
│
├─ README.md                    # Main navigation hub
├─ NEWCOMERS.md                 # Onboarding guide (moved from repo root by Set G G-1.2)
├─ artifact.md                  # Foundational project definition (SC-N, FM-N)
├─ architecture.md              # System architecture & component map
├─ glossary.md                  # Shared terminology
├─ open-questions.md            # Living decision register (Q-N)
├─ docs_home.md                 # Older alternative entry point
├─ parity-matrix.md             # Parity/compatibility tracking
├─ HARNESS_CONFORMANCE.md       # Agent harness capability-profile results (ADR-0031)
├─ LOCAL_TIER_FINDINGS.md       # Local-model tier findings (ADR-0032)
├─ REORGANIZATION_SUMMARY.md    # Historical record of the 2026-06-11 docs reorganization
├─ FOLDER_MAP.md                # This file
│
├─ QUEUE.MD                     # Python-era queue protocol (historical; most rows superseded)
├─ QUEUE_SCHEMA.md              # Queue row schema
├─ QUEUE_ARCHIVE.MD             # Archived queue items
├─ QUEUE_STACK.csv              # Queue stack data file
│
├─ adr/                         # Architecture Decision Records 0001–0048 + README index
│
├─ agent/                       # Agent-facing authoring guides (slv3-guide.md)
│
├─ architecture/                # Technical architecture notes
│   ├─ latency-analysis.md      # (moved from root by Set G G-1.5)
│   ├─ latency-table.md         # (moved from root by Set G G-1.5)
│   ├─ paper_trading.md
│   ├─ monitoring.md, risk_precedence.md, pnl_ledger.md, system_walkthrough.md
│   └─ coinbase_granularity.md, kraken_market_data.md, questdb_traces.md,
│      migration_to_spec_pipeline.md
│
├─ specs/   (on disk: Specs/)   # ~53 files: the canonical spec set
│   ├─ README.md                # Spec index (README.MD case-variant, see note above)
│   ├─ SYS-001 …                # System overview
│   ├─ FEAT-001 … FEAT-007      # Feature specs (FEAT-007 = unified strategy engine, Set U)
│   ├─ COMP-001 … COMP-007      # Component specs
│   ├─ DATA-001 … DATA-006      # Data contracts (DATA-005 data API v2, DATA-006 feature engine)
│   ├─ AGENT-001 … AGENT-014    # Agent runtime, tools, memory, evals, context, Research Desk, modes
│   ├─ INTG-001                 # MCP server
│   ├─ AI_MODELS_SUITE_CAPABILITY_SPEC.md, BACKTEST_SUITE_CORE_SPEC(_V2).md, set-H-api-contract.md
│   └─ *.MD (APP_CONFIG_AND_CONTRACTS, CONTROL_PLANE, DATA_PLANE, …)   # older consolidated specs
│
├─ plans/                       # Every plan lives here
│   ├─ PLAN.md                  # THE one map: read first, updated on every close
│   ├─ ROADMAP.md               # Programme stages, ordered by the evidence clock
│   ├─ EXECUTION-BOARD.md       # Parallel agent-track coordination
│   ├─ OPEN-TASKS.md, TRACK-RULES.md   # Current backlog sweep and its rules
│   ├─ README.md
│   ├─ plan-sets/
│   │   ├─ set-A/               # Original Phase A–7 Rust refactor plans (README + phase files)
│   │   ├─ set-B/               # Registry, paper engine, collectors, frontend (master-plan.md)
│   │   ├─ set-C/               # Latency optimization
│   │   ├─ set-D/               # Backtesting decoupling & hardening
│   │   ├─ set-E/               # Production readiness: stubs, placeholders, tech debt
│   │   ├─ set-F/               # AI agent MCP platform
│   │   ├─ set-G/               # Documentation restructuring (this map is its G-4.1)
│   │   ├─ set-H/               # AI Model Studio
│   │   ├─ set-I/               # AI models: probabilistic forecasting suite
│   │   ├─ set-J/               # Backtest suite: honest-evaluation core
│   │   ├─ set-K/               # Backtest suite: make it real & unify
│   │   ├─ set-L/               # Quant research agent: agent foundation
│   │   ├─ set-P/               # Data reach
│   │   ├─ set-S/               # The agent platform
│   │   ├─ set-T/               # Forward evidence
│   │   └─ set-U/               # Unified strategy engine
│   │                           # (no sets M, N, O, Q or R exist)
│   ├─ reference/               # rust-rewrite-master-plan.md, PHASE_DESIGN_CHECKLIST.md,
│   │                           # PHASE_QUICK_REFERENCE.md
│   └─ special-plans/           # MICROSERVICES_SPLIT_PLAN.MD,
│                               # RESTART_DATA_HEALTH_AND_FORECASTER_PLAN.MD (superseded)
│
├─ ml-platform-context-pack/    # Spec-driven pack for the MLOps/data layer
│   ├─ README.md, CLAUDE.md, GLOSSARY.md, MANIFEST.json
│   ├─ spec/SPEC.md             # Authority order: INVARIANTS > SPEC > reference
│   ├─ decisions/               # INVARIANTS, ADR-INDEX, HANDOFF, PROGRESS, CURRENT-STATE, …
│   ├─ backlog/                 # Acceptance tests, implementation checklist, phase 2–5 plan
│   ├─ reference/               # 00–08 research briefs
│   └─ schemas/                 # 00–06 reference SQL
│
├─ mcp/                         # MCP server notes (README.md)
│
├─ meridian-design-system/      # Meridian UI design system: spec, agent brief, tokens,
│                               # contrast report, screenshots, reference/ HTML mock-ups
│
├─ images/                      # Screenshots used by README files
│
├─ procedures/                  # Atomic step-by-step task instructions
│   ├─ README.md
│   ├─ AGENT.md                 # (moved from root by Set G G-1.1)
│   ├─ add-a-venue, add-adr, add-doc, add-plan, add-research, add-spec
│   ├─ analyze-impact, create-artifact, execute-plan, explain-provenance
│   ├─ operate-the-stack, research-ledger, run-a-backtest-experiment
│   ├─ trace-uncertainty, verify-traceability
│   └─ CI_ROOT_CAUSE_ANALYSIS_PROMPT.md
│
├─ skills/                      # Agent skill definitions that compose procedures (14 files)
│
├─ research/                    # Research briefs & evaluations
│   ├─ README.md, CONCLUSIONS.md (merged from root by Set G G-1.3), OPEN_QUESTIONS.md, SKILL.md
│   ├─ DATA_SOURCES_REFERENCE.md, broker-venue-selection.md, rust-trading-stack-evaluation.md
│   ├─ paper-trading-realism-audit.md, quant-agent-toolbox-uncommon-tools.md,
│   │  strategy-search-study.md
│   └─ prompts/
│
├─ BRAINSTORM/                  # Exploratory design thinking
│   ├─ README.MD, TEMPLATE_BRAINSTORM.MD, AUTOMATION_QUEUE_SLICE_PROMPT.MD
│   ├─ BS-001 … BS-006 (*.MD)
│   └─ BS-007_QUANT_RESEARCH_AGENT/   # 00_INDEX → 17_BUILD_ORDER doc set
│
├─ governance/                  # Audits, release governance, tech-debt log
│   ├─ COMPLACENCY_LOG.md       # (moved from root by Set G G-1.4)
│   └─ audit_code_review.md, full_audit.md, mlflow_promotion.md, release_and_experiments.md
│
├─ operations/                  # Runbooks, deployment, shutdown, rollback
│
├─ reports/                     # Audit reports and coverage matrices (2026-04 to 2026-06)
│
├─ backlog/                     # deferred_roadmap.md
├─ foundation/                  # commentary.md
│
├─ Human Provided Specs/        # User-provided APEX specs (Python era)
│   └─ new_specs/{canonical/, superseded/, MANIFEST.json, README.txt}
│
└─ archive/                     # Superseded docs; see archive/README.md and AUDIT.md
    ├─ specs/                   # Older consolidated spec versions
    └─ canonical-metadata-archive/
```

Outside `docs/`: the repo-root `research/` directory is a stale second research tree
(`CONCLUSIONS.md`, `OPEN_QUESTIONS.md`, `SKILL.md`) shadowing `docs/research/`. Its fate is
an open human decision recorded in Set G MASTER (remainder item 5).

## Quick Navigation

| Need | Location |
|------|----------|
| What to work on now | `plans/PLAN.md`, then `plans/ROADMAP.md` |
| System overview | `artifact.md` + `architecture.md` |
| Architecture decisions | `adr/` (0001–0048, index in `adr/README.md`) |
| Specs | `specs/` (on disk `Specs/`) |
| Implementation plans | `plans/plan-sets/` (set-A … set-U) |
| MLOps/data-layer authority | `ml-platform-context-pack/` |
| How-to procedures | `procedures/` + `skills/` |
| Operational docs | `operations/` |
| UI design system | `meridian-design-system/` |
| Research & exploration | `research/` + `BRAINSTORM/` |
| Open decisions | `open-questions.md` |
| Terminology | `glossary.md` |
