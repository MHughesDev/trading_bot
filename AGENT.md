# AGENT.md

Agent operating instructions for the trading platform repository. Read this file at the start of every session before touching anything.

---

## Mandatory read protocol

Before any code changes, file edits, or task work in this repository:

1. Read `README.md` in full — project overview, current phase, quickstart.
2. Read this file (`AGENT.md`) in full — operating rules, workspace map, traceability discipline.

Re-read at the start of every new session, even after a brief pause. Operational detail changes; memory is not a substitute for the current file.

---

## Repository state

**The Python → Rust refactor is complete (Phase 7 done).** The Rust workspace is the canonical system. The Python codebase (`legacy_python/`) has been deleted. All specs are `Implemented`. The canonical refactor sequence is documented in `docs/plans/rust-rewrite-master-plan.md`.

---

## Workspace map

| Path | What it is |
|------|-----------|
| `docs/` | **The canonical documentation workspace.** Design decisions, specs, ADRs, plans, research, procedures, skills. Start here. |
| `docs/artifact.md` | Project definition — success conditions (SC-N) and failure modes (FM-N) |
| `docs/open-questions.md` | Living register of decisions — Q-N entries |
| `docs/architecture.md` | Current-state system map with repo structure |
| `docs/adr/` | Immutable Architecture Decision Records (ADR-0001 – ADR-0011) |
| `docs/specs/` | Component, data, feature, and integration specs (Status: Implemented) |
| `docs/plans/` | Formal plan copies with Derived From traceability |
| `docs/research/` | Technology evaluations and trade-off briefs |
| `docs/procedures/` | Atomic step-by-step task instructions |
| `docs/skills/` | Agent skill definitions that compose procedures |
| `docs/glossary.md` | Shared terminology |
| `frontend/` | React SPA — kept; re-pointed at Rust endpoints as phases land |

---

## Research-First Protocol

> This protocol is referenced by `docs/procedures/add-spec.md` and all spec-authoring work.

**Before writing or extending any spec, ADR, or plan:** research the design space. Do not write from assumption.

1. Check `docs/research/` for existing briefs on the topic.
2. Check `docs/open-questions.md` for an existing Q-N entry.
3. If neither exists and the decision is consequential, open a Q-N entry in `open-questions.md` before proceeding.
4. For technology choices: search the skills in `docs/skills/` first (`create-research-brief.md`), then draft a research brief in `docs/research/` before writing the spec.

---

## Traceability discipline

Every artifact must trace to its source:

- **Specs** cite their ADR(s) and the SC-N they advance.
- **ADRs** cite the Q-N they resolve and the research brief that informed them.
- **Plans** carry a `Derived From` header listing spec IDs and ADR IDs.
- **Open questions** link their resolving ADR when closed.

Never add a spec ID, SC-N, FM-N, or Q-N without checking `docs/specs/README.md`, `docs/artifact.md`, and `docs/open-questions.md` first — IDs are stable and must not be renumbered.

---

## Non-negotiable invariants (from master plan §2)

These apply to all Rust code written in Phases B–7. Violating them costs real money:

1. **No `f64` on price or size.** Use `domain::money::Price` / `Size` newtypes. The compiler enforces it.
2. **One risk gate, no bypass.** Every order flows through `crates/risk`. No private path to a broker.
3. **Append-only history.** Late data emits a revision event; never mutates published data.
4. **Same builder code live and in replay.** `builders`/`features` are pure functions with no I/O.
5. **`available_time` ordering prevents lookahead.** Replay dequeues strictly by `available_time`.
6. **Idempotency on money-mutating paths.** Fills, risk gate, order submission are keyed for no-op redelivery.
7. **Canonical vs lossy split.** Strategy runtime consumes exact events. UI gateway is intentionally lossy. Runtime never reads the UI feed.
8. **Every "decided mechanism" gets an adversarial test.** A task is not done until its test is green.
9. **Phase 7 is complete.** All specs are `Implemented`, all ACs evidenced, Python deleted, `the reference docs directory` deleted.

---

## Skills-first rule

Before implementing any workspace procedure manually, check `docs/skills/` for an agent skill that covers it. Available skills:

- `create-adr.md` — open a new ADR
- `create-spec.md` — draft a new spec
- `create-plan.md` — create a formal plan
- `create-research-brief.md` — document a technology evaluation
- `create-artifact.md` — initialize or update the artifact
- `execute-plan.md` — run a plan task by task
- `analyze-impact.md` — assess change impact across the workspace
- `explain-provenance.md` — trace why something is the way it is
- `trace-uncertainty.md` — surface open questions from a decision area
- `verify-traceability.md` — check workspace structural integrity
- `generate-system-design.md` — produce a design from artifact + research

---

## Phase A handoff note

**Phase 7 is complete (2026-06-08).** All specs are `Implemented`, all ACs have `Verified by:` evidence, operational procedures are written, and `the reference docs directory` has been deleted.
