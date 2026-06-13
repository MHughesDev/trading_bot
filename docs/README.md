# docs/

All project design content lives here. This folder is the canonical system-design workspace for the trading platform refactor.

---

## Onboarding

| Path | Purpose |
|------|---------|
| [NEWCOMERS.md](./NEWCOMERS.md) | Self-contained learning guide — mental models, module walkthroughs, glossary, FAQ |
| [procedures/AGENT.md](./procedures/AGENT.md) | Agent operating instructions and session conventions |

---

## Architecture & Decisions

| Path | Purpose |
|------|---------|
| [artifact.md](./artifact.md) | Foundational project definition — success conditions (SC-N) and failure modes (FM-N) |
| [architecture.md](./architecture.md) | Current-state system map — components, data flow, repository structure |
| [glossary.md](./glossary.md) | Shared terminology |
| [adr/](./adr/README.md) | Architecture Decision Records — numbered, immutable, indexed (ADR-0001 – ADR-0013) |
| [architecture/](./architecture/) | Latency analysis, granularity docs, system walkthrough |

---

## Specifications

| Path | Purpose |
|------|---------|
| [specs/](./specs/README.md) | Feature, component, data, and integration specifications (Status: Draft/Implemented) |

---

## Plans

| Path | Purpose |
|------|---------|
| [plans/plan-sets/](./plans/plan-sets/) | All implementation plan sets (A through G) |
| [plans/reference/](./plans/reference/) | Phase design checklist and quick-reference docs |
| [plans/special-plans/](./plans/special-plans/) | Microservices split and similar one-off plans |

---

## Governance & Research

| Path | Purpose |
|------|---------|
| [governance/](./governance/) | Audit logs, release governance, complacency tracking |
| [research/](./research/README.md) | Research conclusions, open questions, and research briefs |
| [open-questions.md](./open-questions.md) | Living register of unresolved decisions |
| [backlog/](./backlog/) | Deferred roadmap items |

---

## Operations & Procedures

| Path | Purpose |
|------|---------|
| [procedures/](./procedures/README.md) | Atomic step-by-step task instructions |
| [skills/](./skills/README.md) | Agent skill definitions that compose procedures |
| [operations/](./operations/) | Deployment, runbooks, graceful shutdown, per-asset operator |

---

## Archive

| Path | Purpose |
|------|---------|
| [archive/](./archive/README.md) | Historical Python-era artifacts; UNIQUE files kept for reference |

---

## Status

**Phase A (documentation workspace initialization) is complete. Set G (documentation restructuring) is complete.** All specs are `Status: Draft` or `Status: Implemented`. Phase 7 finalizes draft specs once the Rust system is built.

---

See [README.md](../README.md) for project overview.
