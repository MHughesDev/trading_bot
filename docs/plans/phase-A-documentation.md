---
Type: Formal
Status: Current
Derived From: SYS-001, DATA-001, DATA-002, DATA-003, DATA-004, FEAT-001, COMP-001, COMP-002, COMP-003, COMP-004, INTG-001, ADR-0001, ADR-0002, ADR-0003, ADR-0004, ADR-0005, ADR-0006, ADR-0007, ADR-0008, ADR-0009, ADR-0010, ADR-0011
Note: Canonical executable plans live in refactor_reference_docs/plans/. This copy is the traceable documentation record. On any conflict, refactor_reference_docs/ wins.
---

# Phase A — Initialize documentation workspace

> **Self-contained execution doc.** You need only: this file, the template at
> `../template/docs/`, the existing reference material in
> `refactor_reference_docs/spec/`, `refactor_reference_docs/file-structure.md`, and the plan files in
> `refactor_reference_docs/plans/`.
>
> **This phase runs BEFORE everything else** — before any Rust is written, before the workspace is
> scaffolded (Phase B). The user always structures project documentation using the template `docs/`
> folder. Initializing the workspace first means every later phase is authored, traced, and executed
> from inside that structure.
>
> **This is the INITIALIZE pass.** It captures the design-time picture — what we intend to build and
> why. Specs are seeded in `Draft` status with acceptance criteria marked `Verified by: [—]` (unfilled).
> The finalize pass happens in **Phase 7**, once all system files actually exist, and is when specs
> advance to `Implemented`, evidence is recorded, and operational procedures are written from reality
> rather than intent.

## Phase goal

After this phase, the repository has a single canonical **`docs/` system-design workspace at the
repo root**, scaffolded from the template and initialized with the full
design-time knowledge base: the artifact, the open-questions register, the architecture map, the ADRs
capturing every already-decided choice, the specs (migrated from `spec/` as `Draft`), the research
briefs, the plans (this entire refactor, migrated in with **Derived From** traceability), and the
`procedures/`+`skills/` tooling so every future doc follows the same conventions.

**What Phase A does NOT do:** mark specs `Implemented`, fill in `Verified by:` evidence, write
operational runbooks from a real running system, or finalize the architecture map to the as-built
state. All of that is **Phase 7's job**, once the system is built.

This phase **only moves and restructures documentation** — it writes no system code.

## Prerequisites

- None. This is the first phase of the whole effort.
- Read the template thoroughly first: the template's `docs/README.md` and
  every folder `README.md` (`adr/`, `specs/`, `plans/`, `procedures/`, `research/`, `skills/`) define
  the conventions you must follow exactly (spec ID scheme `<TYPE>-<NNN>`, ADR numbering, the
  artifact's `SC-N`/`FM-N` IDs, the open-questions `Q-N` register, the plan **Derived From**
  traceability).

## Invariants this phase must respect

- **Follow the template's conventions exactly.** Spec IDs are `<TYPE>-<NNN>`; ADRs are
  `NNNN-title.md` and immutable; success conditions are `SC-N`, failure modes `FM-N`, open questions
  `Q-N`; never renumber, always append.
- **Traceability is the point.** Every spec cites the ADR(s) and research that informed it; every
  plan task traces to a spec section or architecture node (**Derived From**); every open question
  records options, resolution, and evidence. The end state must pass
  [`../procedures/verify-traceability.md`](../procedures/verify-traceability.md).
- **Nothing is deleted that carries history.** Specs and ADRs retire (`Deprecated`/`Superseded`),
  they are not deleted.
- **`refactor_reference_docs/` is read-only and permanent until the very end.** It stays at the
  repo root, untouched, throughout the entire refactor — Phases A through 6. It is the permanent
  reference anchor. **Never move it, never modify it, never delete it during this phase.** The only
  time it is deleted is as the **very last task in Phase 7**, after everything else is verified done.
  Deleting it is the closing signal that the refactor is complete.
- **The workspace is the design record, not the code.** `docs/` holds design/decision/plan content;
  it is not where system code or runtime config lives.

---

## Tasks

### P A-T01 — Scaffold `docs/` from the template
- **Goal:** Create the repo-root `docs/` workspace as a copy of the template's structure and
  meta-tooling, with example artifacts removed.
- **Files:** copy the template's `docs/` → repo-root `docs/`. Keep verbatim:
  `docs/procedures/*` (all 10), `docs/skills/*` (all 11), every folder `README.md`. Keep as
  empty-but-templated starting files: `docs/artifact.md`, `docs/open-questions.md`,
  `docs/architecture.md`. **Delete the example artifacts:** `adr/0001-example-*`,
  `specs/FEAT-001-user-authentication.md`, `research/example-*`, `plans/example-*`, and the example
  rows in `open-questions.md` and each index README.
- **Context:** The `procedures/` and `skills/` files are project-agnostic source-of-truth tooling —
  copy them unchanged. The template README references a repo-root `AGENT.md` and `README.md`
  (created in P A-T09).
- **Acceptance:** `docs/` exists at repo root with the full folder set; all example artifacts and
  example index rows are gone; procedures/skills are present and unmodified.
- **Depends on:** none.

### P A-T02 — Write the artifact (`docs/artifact.md`)
- **Goal:** Fill the foundational project definition from existing material, assigning stable
  `SC-N`/`FM-N` IDs that downstream specs/plans will cite.
- **Files:** `docs/artifact.md`.
- **Context:** Source the content from `refactor_reference_docs/spec/00-overview.md` and
  the root `README.md`: *What we're building* (the event-driven local-first trading
  platform), *Who uses it* (the trusted small group), *What problem it solves* (correct, trustworthy
  money handling), *What good looks like* → derive `SC-1…SC-N` from the spec's success properties
  (e.g. SC: "no `f64` ever touches a price"; SC: "same strategy code, same backtest result"; SC:
  "every order passes one risk gate"; SC: "a new asset class is a collector + payload + metadata
  rows, not a redesign"). *What would make it fail* → `FM-1…FM-N` from the reconciliation/lookahead/
  double-submit failure modes in the spec. *Not building* → the deliberate non-goals (twelve
  microservices, multi-tenant isolation, Kafka, every asset class at once). *Working within* →
  Rust, local-first, small team.
- **Acceptance:** `artifact.md` has no placeholder text; `SC-N` and `FM-N` are numbered and stable;
  every claim traces to the overview/README.
- **Depends on:** P A-T01.

### P A-T03 — Migrate the open-questions register (`docs/open-questions.md`)
- **Goal:** Port the spec's open questions into the `Q-N` register with status, options, resolution,
  and evidence.
- **Files:** `docs/open-questions.md`.
- **Context:** From `refactor_reference_docs/spec/10-open-questions.md`: Q1 (real-vs-paper),
  Q2 (broker/venue), Q3 (strategy-format freeze), Q4 (capital/liability), Q5 (auth), Q6 (backtest
  fidelity), Q7 (watermark defaults), Q8 (retention) become `Q-1…Q-8`. Mark **Q-1, Q-2, and Q-3** as
  **Resolved** with the deciding ADR linked (created in P A-T04); the rest **Open** with their options
  recorded. Q-1 resolution: Alpaca paper account for paper trading on all assets/domains (ADR-0006).
  Q-2 resolution: Coinbase=live, Alpaca=paper+equity-data, Kraken=crypto-data,
  market_simulator=backtest; venue routing via `crates/venue-router`; pipelines start on demand only
  (ADR-0006, ADR-0011). Q-3 resolution: format frozen in Phase 0 (ADR-0007).
- **Acceptance:** every spec/plan reference to a decision maps to a `Q-N` row; resolved rows link
  their ADR; no example rows remain.
- **Depends on:** P A-T01.

### P A-T04 — Capture decided architecture as ADRs (`docs/adr/`)
- **Goal:** Record every already-made architectural decision as an immutable ADR using the template's
  ADR format, and index them.
- **Files:** `docs/adr/NNNN-*.md` (one per decision below) + `docs/adr/README.md` index rows.
- **Context:** Mine the specs for the decisions and write one ADR each (use the ADR template in
  `docs/adr/README.md`: Context/Decision/Rationale/
  Consequences/Alternatives). Suggested set (number sequentially, adjust as needed):
  - `0001-rust-modular-monolith-with-satellite-collectors` (from `refactor_reference_docs/spec/01-architecture.md`)
  - `0002-decimal-money-newtypes-no-f64` (from `refactor_reference_docs/spec/02-data-model.md`/`03-data-engineering.md`)
  - `0003-nats-jetstream-event-fabric` (from `refactor_reference_docs/spec/09-tech-stack.md`)
  - `0004-storage-split-postgres-clickhouse-parquet-redis` (from `refactor_reference_docs/spec/07-storage-and-replay.md`)
  - `0005-single-risk-gate-chokepoint-and-kill-switch` (from `refactor_reference_docs/spec/05-execution-and-risk.md`)
  - `0006-three-system-broker-architecture-coinbase-alpaca-market-simulator` (resolves Q-1 AND Q-2:
    Coinbase=live execution all assets, Alpaca=paper execution all assets, market_simulator=backtest
    execution; Kraken=crypto market data, Alpaca data feed=equity market data; three parallel systems
    must exist for all assets/domains; from `refactor_reference_docs/spec/05-execution-and-risk.md`/`10-open-questions.md`)
  - `0011-demand-driven-data-engines-no-auto-start` (data pipelines start ONLY when a strategy or UI
    panel declares demand via the Demand Manager + venue-router; never on system init; from
    `refactor_reference_docs/spec/03-data-engineering.md`/`04-strategy-system.md`)
  - `0007-freeze-strategy-definition-format-v1` (resolves Q-3, from `refactor_reference_docs/spec/04-strategy-system.md`)
  - `0008-available-time-ordering-and-same-builders-live-and-replay` (from `refactor_reference_docs/spec/03-data-engineering.md`/`07-storage-and-replay.md`)
  - `0009-append-only-raw-event-archive-as-ground-truth` (from `refactor_reference_docs/spec/07-storage-and-replay.md`)
  - `0010-three-front-doors-one-canonical-strategy-json` (from `refactor_reference_docs/spec/04-strategy-system.md`/`08-mcp-server.md`)
- **Acceptance:** each ADR is `Accepted`, follows the template sections, and lists alternatives; the
  `adr/README.md` index has a row per ADR; Q-1/Q-3 in open-questions link to ADR-0006/ADR-0007.
- **Depends on:** P A-T03.

### P A-T05 — Migrate the spec set (`docs/specs/`)
- **Goal:** Convert the `spec/` documents into template specs with `<TYPE>-<NNN>` IDs and the fixed
  seven-section structure, each citing its ADR(s) and the artifact `SC-N` it serves.
- **Files:** `docs/specs/<TYPE>-<NNN>-*.md` + `docs/specs/README.md` index. Use the spec template and
  procedure in [`../procedures/add-spec.md`](../procedures/add-spec.md).
- **Context:** Recommended mapping (split further if a file would exceed ~7 cohesive sections):
  - `refactor_reference_docs/spec/02-data-model.md` → `DATA-001` event-envelope-and-payloads,
    `DATA-002` instrument-metadata, `DATA-003` timestamps-and-identity.
  - `refactor_reference_docs/spec/03-data-engineering.md` → `COMP-001` data-quality-and-ingestion
    (quarantine, dedup, watermarks/revisions, reconciliation policy).
  - `refactor_reference_docs/spec/04-strategy-system.md` → `FEAT-001` strategy-system +
    `DATA-004` strategy-definition-format (the frozen 1.0 contract).
  - `refactor_reference_docs/spec/05-execution-and-risk.md` → `COMP-002` execution-and-risk-gate.
  - `refactor_reference_docs/spec/06-ui-and-streaming.md` → `COMP-003` ui-streaming-gateway.
  - `refactor_reference_docs/spec/07-storage-and-replay.md` → `COMP-004` storage-and-replay.
  - `refactor_reference_docs/spec/08-mcp-server.md` → `INTG-001` mcp-server.
  - A `SYS-001` system-overview spec linking all of the above (diagram + cross-reference index),
    sourced from `refactor_reference_docs/spec/01-architecture.md`.
  - `refactor_reference_docs/spec/09-tech-stack.md` becomes a **research brief** (P A-T06) + the ADRs,
    not a spec. `refactor_reference_docs/spec/12-glossary.md` becomes `docs/glossary.md` (P A-T09).
  Each spec's §1.3 links its ADR(s); §6 acceptance criteria are seeded from the spec's "decided
  mechanism" tests with `Verified by: [—]` and **checkboxes unticked** — this is correct for
  initialize. Filling those in is Phase 7's job. §7 references the relevant `Q-N`.
- **Acceptance:** every `spec/` document is represented by one or more template specs; all are
  `Status: Draft`; each has all seven sections, a Non-Goals subsection, ADR links, and an index row;
  spec IDs follow the scheme; no acceptance criterion has `Verified by:` filled in (that is correct).
- **Depends on:** P A-T04.

### P A-T06 — Write the research briefs (`docs/research/`)
- **Goal:** Capture the evaluations that justified the stack and the gating venue choice.
- **Files:** `docs/research/rust-trading-stack-evaluation.md`,
  `docs/research/broker-venue-selection.md`, + `docs/research/README.md` index rows.
- **Context:** The first brief sources `refactor_reference_docs/spec/09-tech-stack.md`
  (Question/Method/Findings/Recommendation/References per the research README) and is cited by
  ADR-0003/0004 and the tech ADRs. The second brief documents the **resolved** broker/venue
  selections for Q-1 and Q-2: Coinbase=live execution (all assets), Alpaca paper account=paper
  execution (all assets), market_simulator (github.com/MHughesDev/market_simulator)=backtest
  execution, Kraken WS=crypto market data, Alpaca data feed=equity market data. Record rationale,
  alternatives considered, and the demand-driven pipeline constraint. Cite ADR-0006 + ADR-0011.
- **Acceptance:** both briefs follow the research format; the tech brief is cited by the stack ADRs;
  the venue brief is linked from Q-1 and Q-2 (both Resolved); index rows added.
- **Depends on:** P A-T04.

### P A-T07 — Write the architecture map (`docs/architecture.md`)
- **Goal:** The current-state structural map — including the enumerated end-state file structure.
- **Files:** `docs/architecture.md`.
- **Context:** Fill the template's sections (Overview, Components, Data Flow, External Dependencies,
  Key Decisions, Constraints) from `refactor_reference_docs/spec/01-architecture.md`. **Fold
  the entire enumerated structure from `refactor_reference_docs/file-structure.md` into the
  Components section** (or a dedicated "Repository structure" section) — this is exactly what
  `architecture.md` is for: the single answer to "what does the system look like and how do the
  pieces fit." Components reference their `COMP/DATA/FEAT` spec; Key Decisions link the ADRs; External
  Dependencies link the tech research brief + ADRs.
- **Acceptance:** `architecture.md` has no `[not yet decided]` left where a spec/ADR has decided it;
  the crate/app/file structure is enumerated here; every component row links a spec; Key Decisions
  link ADRs.
- **Depends on:** P A-T05, P A-T06.

### P A-T08 — Migrate the refactor plans (`docs/plans/`)
- **Goal:** Move this entire refactor (master plan + every phase) into `docs/plans/` as **Formal**
  plans with the template's **Derived From** traceability, re-pointing all links to the new `docs/`
  locations.
- **Files:** `docs/plans/rust-rewrite-master-plan.md` (from
  `refactor_reference_docs/plans/00-master-plan.md`); `docs/plans/phase-A-documentation.md` …
  `phase-7-cutover.md` (from `refactor_reference_docs/plans/`); `docs/plans/README.md` index rows. Follow
  [`../procedures/add-plan.md`](../procedures/add-plan.md).
- **Context:** Each migrated plan gets a header with `Type: Formal`, `Status: Current`, and a
  **Derived From** section listing the specs/ADRs/`SC-N` it traces to **at spec-ID granularity**
  (e.g. "Phase 0 ← `DATA-001`, `DATA-002`, `DATA-003`, `DATA-004`, ADR-0002, ADR-0007, SC-1, SC-2";
  "Phase 2 ← `COMP-002`, ADR-0005, ADR-0006"). Update the internal links that point at
  `../spec/*`/`../file-structure.md` to the new `docs/specs/*`/`docs/architecture.md` targets.
  **Section-level traces (`§N.M`) are best-effort, not required here** — spec-ID granularity is
  enough for the initialize pass. Per-criterion (`§6.1.x`) coverage mapping is completed in Phase 7's
  finalize, when the specs are `Implemented`. Do **not** hand-map every task to a spec section now;
  that precision is not needed to start work and is error-prone.
  > Note: these `docs/plans/` copies are the **traceable record**. The canonical executable plans
  > stay in `refactor_reference_docs/plans/` and are what later phases are run from (see master plan
  > §3.1). Keep the two consistent in intent; the `refactor_reference_docs/` copy wins on conflict.
- **Acceptance:** all plans exist under `docs/plans/`; each is `Formal` with a populated **Derived
  From** at spec-ID granularity; the `../spec/`/`../file-structure.md` links are re-pointed to
  `docs/`; index rows added. (Section-level `§N.M` traces are optional at this stage.)
- **Depends on:** P A-T07.

### P A-T09 — Root README, AGENT.md, and glossary
- **Goal:** The repo-root entry points the template expects, plus the glossary.
- **Files:** repo-root `README.md` (or update existing), repo-root `AGENT.md`, `docs/glossary.md`.
- **Context:** `README.md` = project overview pointing into `docs/`. `AGENT.md` = agent operating
  instructions (the Research-First Protocol referenced by `add-spec.md`, the "search `skills/` first"
  rule from the skills README, and the traceability discipline). `glossary.md` = the terms from
  [`../glossary.md`](../glossary.md). Note: the legacy root `AGENTS.md` from the
  Python system is swept into `legacy_python/` in Phase B — the new `AGENT.md` is the workspace's.
- **Acceptance:** `README.md` and `AGENT.md` exist at root and reference `docs/`; `glossary.md`
  carries the spec's terms; the template's `../AGENT.md`/`../README.md` references now resolve.
- **Depends on:** P A-T08.

### P A-T10 — Verify structural integrity of the initialized workspace
- **Goal:** Prove the initialized workspace is internally consistent — all references resolve, no
  broken links, indexes complete. `refactor_reference_docs/` is **not** touched here; it remains at
  root throughout the entire refactor.
- **Files:** run [`../procedures/verify-traceability.md`](../procedures/verify-traceability.md)
  steps 1–4 and 7–8 (the structural checks) against `docs/`. Skip step 9 ("Verification evidence on
  implemented specs") entirely — at initialize every spec is `Draft` with empty `Verified by: [—]`
  fields, and that is correct and intentional. Record the report.
- **Context:** Check: every `Q-N`/`SC-N`/`FM-N`/`ADR-NNNN`/`§N.M` reference resolves; every plan
  passes two-way coverage (tasks have a Derived From source); no broken links; indexes match folders.
  If anything fails, fix it in `docs/`. Do not touch `refactor_reference_docs/`.
- **Acceptance:** structural traceability checks (steps 1–4, 7–8) pass; all specs are `Draft` with
  unfilled evidence (correct); `refactor_reference_docs/` is still present and unmodified at root;
  `docs/` is the sole canonical workspace from which Phases B–6 are authored and executed.
- **Depends on:** P A-T09.

---

## Phase exit criteria (initialize pass)

- [ ] Repo-root `docs/` exists, scaffolded from the template, with `procedures/` + `skills/` intact
      and all example artifacts removed.
- [ ] `artifact.md` (with `SC-N`/`FM-N`), `open-questions.md` (`Q-1…Q-8`), and `architecture.md`
      (including the enumerated file structure) are populated from the reference material.
- [ ] Every decided choice is an `Accepted` ADR (11 ADRs total, including ADR-0006 three-system
      broker architecture and ADR-0011 demand-driven pipelines); every `spec/` document is migrated
      to a template spec with `<TYPE>-<NNN>` ID and `Status: Draft`; the tech + venue research briefs exist.
- [ ] **All spec acceptance criteria are `Verified by: [—]` with unchecked boxes** — this is
      intentionally correct at initialize. Evidence and implementation status are Phase 7's job.
- [ ] The whole refactor (master + all phase plans) lives in `docs/plans/` as Formal plans with
      **Derived From** traceability and resolving links.
- [ ] Root `README.md` + `AGENT.md` + `docs/glossary.md` exist.
- [ ] Structural traceability (steps 1–4, 7–8 of `verify-traceability`) passes; `docs/` is the
      single source of documentation truth that Phases B–6 are authored and executed from.
- [ ] **`refactor_reference_docs/` is still present and unmodified at the repo root** — it is never
      touched during the refactor and is only deleted as the very last act of Phase 7.
- [ ] **Handoff note logged:** "Phase 7 finalizes these docs — advance specs to `Implemented`,
      fill `Verified by:` evidence, add operational procedures from reality, run the full
      verify-traceability (including step 9), then delete `refactor_reference_docs/` as the final act."
