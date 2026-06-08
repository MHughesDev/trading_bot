# docs/

All project design content lives here. This folder is the canonical system-design workspace for the trading platform refactor.

---

## Contents

| Path | Purpose |
|------|---------|
| [artifact.md](./artifact.md) | Foundational project definition — success conditions (SC-N) and failure modes (FM-N) |
| [open-questions.md](./open-questions.md) | Living register of decisions — Q-N entries, options weighed, resolutions |
| [architecture.md](./architecture.md) | Current-state system map — components, data flow, repository structure |
| [glossary.md](./glossary.md) | Shared terminology |
| [adr/](./adr/README.md) | Architecture Decision Records — numbered, immutable, indexed (ADR-0001 – ADR-0011) |
| [plans/](./plans/README.md) | Formal plan copies with Derived From traceability (all 11 refactor phases) |
| [procedures/](./procedures/README.md) | Atomic step-by-step task instructions |
| [research/](./research/README.md) | Research briefs — technology evaluations and trade-off analyses |
| [skills/](./skills/README.md) | Agent skill definitions that compose procedures |
| [specs/](./specs/README.md) | Feature, component, data, and integration specifications (Status: Draft) |

---

## Status

**Phase A (documentation workspace initialization) is complete.** All specs are `Status: Draft` with `Verified by: [—]` — this is correct and intentional. Phase 7 finalizes them once the Rust system is built.

The canonical executable plans live in `refactor_reference_docs/plans/` at the repo root (read-only reference anchor). The copies in `plans/` here are the traceable documentation record. On any conflict, `refactor_reference_docs/` wins.

---

See [README.md](../README.md) for project overview and [AGENT.md](../AGENT.md) for agent operating instructions.
