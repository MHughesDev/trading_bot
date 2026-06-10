---
Type: Formal
Status: Pending
Derived From: C-043, C-065, C-101, C-102, C-103, C-106, C-110
---

# Phase 7 — Graph, Social & Polish

> **Self-contained execution doc.** You need only: this file, [`../../architecture.md`](../../architecture.md),
> the specs under [`../../specs/`](../../specs/), and the existing codebase. Phase 2 stood up the
> TigerGraph + Milvus containers and client crate skeletons (`crates/graph`, `crates/semantic`) and
> the Reddit collector. This phase populates and uses them, adds the web scraper satellite, and
> finalizes docs. Read `crates/graph`, `crates/semantic`, the registries from Phase 1, and
> `NEWCOMERS.md` before editing.

## Phase goal

After this phase the **knowledge layer is live and the system is polished for handoff**: TigerGraph
holds the **capability/compatibility graph** (rebuildable from Postgres/code registries); Milvus holds
**embeddings** for Reddit posts, web-page snapshots, and strategy descriptions, with metadata-filtered
semantic search; the **web scraper satellite** ingests `web.page_snapshot` facts with `robots.txt`
compliance; `NEWCOMERS.md` reflects the end-state; frontend lint is cleaned where safe; and the specs
for completed phases advance from `Draft` to `Implemented`.

## Prerequisites

- Phases 1–6 done.
- Phase 2: TigerGraph + Milvus containers run; `crates/graph` and `crates/semantic` connect/ping; the
  Reddit collector emits `social.post` events.
- An OpenAI API key is configured for `text-embedding-3-small` (1536 dims).

## Invariants this phase must respect

- **TigerGraph is derived and rebuildable** (C-102): the graph is a projection of Postgres/code
  registries — never an independent source of truth. A rebuild must reproduce it exactly.
- **Milvus is metadata-filtered semantic search** (C-103): embeddings are searched with metadata
  filters; it is not a system-of-record for the text.
- **Scraper compliance** (C-101): `robots.txt` is honored and per-domain rate limits apply.
- **Minimum source data / no risk UI / no backtest** carry over from prior phases.

---

## Tasks

### P7-T01 — TigerGraph schema initialization
- **Goal:** Define and create the capability/compatibility graph schema.
- **Files:** `crates/graph/src/schema.rs` (new), a GSQL/schema definition file
  `crates/graph/schema/capability_graph.gsql` (new).
- **Context:** Per C-102/C-065. Vertices: `AssetClass`, `Instrument`, `Venue`, `DataType`,
  `StrategyDefinition`, `Widget`. Edges: `INSTRUMENT_IS_A` (Instrument→AssetClass), `VENUE_PROVIDES`
  (Venue→DataType), `STRATEGY_REQUIRES_DATA` (StrategyDefinition→DataType), plus
  instrument/venue/asset-class linkage edges needed for compatibility queries. The schema is created
  idempotently via the client.
- **Acceptance:** `crates/graph/tests/schema_init.rs` creates the schema against the compose
  TigerGraph and confirms all vertex and edge types exist; re-running is a no-op (idempotent).
- **Depends on:** Phase 2 TigerGraph client.

### P7-T02 — Graph population from registries (derived, rebuildable)
- **Goal:** Populate the graph from Postgres/code registries and prove a rebuild is exact.
- **Files:** `crates/graph/src/populate.rs` (new), a `platform` admin command/route to trigger rebuild.
- **Context:** Per C-102. Read `asset_class_registry`, `data_type_registry`, instruments,
  `SupportedVenue` capabilities, and saved strategies/manifests; upsert vertices and edges. A full
  rebuild drops and re-derives the graph from the registries — **the graph carries no data that isn't
  derivable** from Postgres/code.
- **Acceptance:** `crates/graph/tests/rebuild_exact.rs` populates, snapshots vertex/edge counts and a
  sample compatibility query result, wipes, rebuilds, and asserts the snapshot is identical.
- **Depends on:** P7-T01.

### P7-T03 — Milvus collection + embedding pipeline
- **Goal:** Create the Milvus collection and an embedding pipeline for social/web/strategy text.
- **Files:** `crates/semantic/src/embed.rs` (new), `crates/semantic/src/collection.rs` (new),
  `apps/embedder/` (new satellite that consumes text events and writes embeddings).
- **Context:** Per C-103/C-065/C-106/C-101. Collection schema: 1536-dim vector
  (`text-embedding-3-small`) + metadata (source ∈ {`social.post`,`web.page_snapshot`,
  `strategy.description`}, instrument ids, venue, timestamps). The embedder consumes `social.post`
  (Reddit), `web.page_snapshot` (scraper), and strategy descriptions, calls OpenAI embeddings, and
  upserts vectors with metadata. Search is metadata-filtered.
- **Acceptance:** `crates/semantic/tests/embed_search.rs` embeds three sample texts (one per source),
  upserts them, and proves a metadata-filtered semantic search (e.g. `source = social.post` near a
  query) returns the expected post and excludes the other sources.
- **Depends on:** Phase 2 Milvus client + Reddit collector.

### P7-T04 — Web scraper satellite
- **Goal:** A satellite that scrapes web pages and emits `web.page_snapshot` facts, compliantly.
- **Files:** `crates/collectors/src/web/scraper.rs` (new), `apps/collector-web/` wiring (new).
- **Context:** Per C-101. **HTTP-first**, with a **Playwright fallback** for JS-rendered pages.
  Enforce **`robots.txt`** compliance and **per-domain rate limits** (admit via the Phase 2 rate
  budget). Extracted page facts are emitted as `DataType::WebPageSnapshot` (`web.page_snapshot`)
  events on the bus; they are embedded into Milvus by the P7-T03 embedder.
- **Acceptance:** `crates/collectors/tests/scraper_compliance.rs` proves a `robots.txt`-disallowed
  path is skipped, a per-domain rate limit throttles requests, and a fetched page produces a
  `web.page_snapshot` event; the Playwright fallback triggers only when HTTP yields no usable content.
- **Depends on:** Phase 2 rate budget + bus.

### P7-T05 — NEWCOMERS.md update
- **Goal:** `NEWCOMERS.md` reflects the end-state system.
- **Files:** `NEWCOMERS.md`.
- **Context:** Per C-110/C-043. Update the onboarding doc to describe: the Rust monolith + satellite
  collectors, the 8 asset classes and venues, the PAPER/LIVE_ROUTED execution model with internal
  paper simulators, the event-sourced ledger + USD rollup, the five frontend sections, the
  collector-sharing/DemandRegistry model, and the TigerGraph/Milvus knowledge layer. Remove stale
  references (backtesting, Alpaca paper, risk UI, order books).
- **Acceptance:** `NEWCOMERS.md` accurately describes the end-state with no mention of removed concepts
  (backtest, Alpaca paper, risk settings, order book); a reviewer can follow it to run the stack.
- **Depends on:** Phases 1–6.

### P7-T06 — Frontend lint cleanup (safe-only)
- **Goal:** Clean frontend lint where it does not risk behavior changes.
- **Files:** `frontend/` (lint-flagged files), `frontend/.eslintrc*` if needed.
- **Context:** The frontend has pre-existing deferred lint errors. Fix the **safe** ones (unused
  imports/vars, formatting, obvious type holes) **without** altering component behavior. Leave any lint
  fix that would require behavioral change documented in the PR rather than risking a regression.
- **Acceptance:** `npm run lint` in `frontend/` reports materially fewer errors with **no** behavior
  change; `npm run build` still succeeds; any intentionally-deferred lint is listed in the PR.
- **Depends on:** Phases 5–6.

### P7-T07 — Docs finalize: advance specs Draft → Implemented
- **Goal:** Advance the specs for completed phases to `Implemented` with evidence, and reconcile docs.
- **Files:** the relevant specs under `docs/specs/` (e.g. COMP-001/002/003, DATA-002/004, FEAT-001),
  `docs/architecture.md`, this plan set's `README.md` statuses.
- **Context:** Per C-043. For each spec whose behavior is now built (collectors/ingestion, execution,
  UI streaming, instrument metadata, strategy system), change `Status: Draft` to `Implemented` and add
  a `Verified by:` reference to the phase tasks/tests that prove it. Reconcile `architecture.md` to the
  as-built crate/file layout (new crates: `graph`, `semantic`; new collector/exec venue modules; the
  retired Alpaca paper path). Flip the set-B plan statuses to `Done` where exit criteria are green.
- **Acceptance:** every spec for a completed phase reads `Implemented` with a `Verified by:` pointer;
  `architecture.md` lists the new crates and the removed Alpaca-paper path; the set-B README reflects
  final statuses.
- **Depends on:** all prior tasks.

---

## Phase exit criteria
- [ ] TigerGraph schema (the listed vertices/edges) is created idempotently.
- [ ] The graph is populated from registries and a wipe+rebuild reproduces it exactly (derived,
      rebuildable).
- [ ] Milvus collection + embedding pipeline embed social/web/strategy text; metadata-filtered semantic
      search returns the right source.
- [ ] The web scraper satellite honors `robots.txt` + per-domain limits, uses HTTP-first with
      Playwright fallback, and emits `web.page_snapshot` events.
- [ ] `NEWCOMERS.md` describes the end-state with no removed concepts.
- [ ] Frontend lint is reduced with no behavior change; `npm run build` succeeds.
- [ ] Specs for completed phases read `Implemented` with `Verified by:`; `architecture.md` reconciled;
      set-B README statuses updated.
- [ ] `cargo check --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
      --all-features` all green.
