---
Type: Formal
Status: Current
Derived From: SYS-001, DATA-001, DATA-002, DATA-003, DATA-004, FEAT-001, COMP-001, COMP-002, COMP-003, COMP-004, INTG-001, ADR-0001, ADR-0002, ADR-0003, ADR-0004, ADR-0005, ADR-0006, ADR-0007, ADR-0008, ADR-0009, ADR-0010, ADR-0011, SC-1, SC-2, SC-3, SC-4, SC-5, SC-6, SC-7
Note: Canonical executable plans live in docs/plans/. This copy is the traceable documentation record. On any conflict, [deleted - see Phase 7]/ wins.
---

# Master Refactor Plan — Python → Rust Trading Platform

> **This is the top-level plan.** It defines the goal, the rules every phase obeys, the phase
> sequence, and how the work is tracked. Each phase has its own self-contained plan file in this
> directory. Read this file first, then execute one phase file at a time, in order.
>
> **➤ New here / about to start implementing? Jump to [§3.1 START HERE](#31-start-here--execution-entry-point-read-this-before-doing-anything).**
> It tells you which file to execute, in what order, and how to track progress. The first thing you
> will execute is [`phase-A-documentation.md`](./phase-A-documentation.md).

---

## 0. What we are doing and why

The current repository is a large, organically-grown **Python** trading system (`app/`, `services/`,
`control_plane/`, `data_plane/`, `execution/`, `risk_engine/`, `training_pipeline/`, `legacy/`, …).
It works but the structure is messy, overlapping, and hard to reason about for a money-handling
system.

We are doing **two things at once**:

1. **Rewriting the entire backend in Rust**, following the architecture in [`../specs/`](../specs/).
2. **Restructuring** into a clean, procedural Cargo workspace defined in
   [`../architecture.md`](../architecture.md).

The React frontend (`frontend/`) is **kept** and re-pointed at the new Rust API/WS contracts.

The end state is the file/folder structure enumerated in
[`../architecture.md`](../architecture.md). That document is the **structural contract**; this
plan and the phase files are the **execution contract** for getting there.

---

## 1. The one principle (carried from the spec)

> Build the cheapest **correct** system one small team can fully trust with real money, where the
> parts most expensive to get wrong — event schema, timestamp semantics, money/ledger model, the
> risk gate, the strategy definition format — are decided first and everything else stays
> changeable. Correctness here is overwhelmingly a **data-quality** property.

This is why the phases are ordered the way they are: the **irreversible core comes first**, money
safety comes before any automation, and the fun streaming/UI work stands on proven ground.

---

## 2. Rules every phase obeys (non-negotiable invariants)

These are restated in every phase file too, because each phase file is meant to be executed
standalone. They are the things that, if violated, cost real money:

1. **No `f64` ever touches a price or size.** Use `domain::money::Price` / `Size` (newtypes over
   `Decimal`, no `From<f64>`). The compiler enforces it; do not add a `From<f64>` to bypass.
2. **One risk gate, no bypass.** Every order — manual or strategy-emitted — flows through
   `crates/risk`. The strategy runtime and the UI never have a private path to a broker.
3. **Append-only, never rewrite history.** Late data emits a **revision event**; it never mutates a
   published bar/row. The raw normalized event archive is immutable ground truth.
4. **Same builder code live and in replay.** `builders`/`features` are pure functions with no I/O,
   fed by the live bus or by the replay loader — never two implementations.
5. **`available_time` ordering prevents lookahead.** The replay loop dequeues strictly by
   `available_time`; it is structurally impossible to hand a strategy its own future.
6. **Idempotency on every money-mutating path.** Fills, the risk gate, and order submission are
   keyed so a redelivery is a no-op. Reconnects and JetStream redelivery **will** double-deliver.
7. **Canonical vs lossy split.** The strategy runtime and storage consume exact, ordered events.
   The UI gateway is a separate, intentionally lossy consumer view. The runtime never reads the UI
   feed.
8. **Every "decided mechanism" gets an adversarial test that proves it fires.** Quarantine,
   revisions, idempotent fills, no-lookahead, reconciliation halts, kill-switch trips, tighten-only
   risk overrides. "Decided" ≠ "done"; a phase task is not complete until its test is green.
9. **`[deleted - see Phase 7]/` stays at the repo root, read-only and unmodified, throughout the
   entire refactor.** It is the permanent reference anchor (spec, architecture, file structure, plans
   as originally authored). Never move it, rename it, or modify any file inside it during Phases A–6.
   Deleting it is the **very last act of Phase 7** (P7-T07), after everything else is verified done.
   Its deletion is the closing signal that the refactor is complete.

---

## 3. How the phases are sequenced

The sequence mirrors the roadmap spec, with a **documentation phase
and** a bootstrap phase added at the front and a cutover phase added at the back to account for the
Python→Rust nature of the work.

**Documentation comes first (Phase A).** Before any Rust is written, the entire documentation set is
refactored into the canonical `docs/` template workspace (the structure the user uses for every
project: `artifact` → `research` → `open-questions` → `adr` → `specs` → `plans`, with
`procedures`/`skills` tooling). Every later phase (B, 0–7) is then authored, traced, and executed
from inside that workspace — the plan files migrate into `docs/plans/` with full **Derived From**
traceability to the specs and ADRs. `docs/` is the documentation/design record; it is **not** where
system code is added.

| Phase | File | Theme | Gate to start |
|-------|------|-------|----------------|
| **A** | [`phase-A-documentation.md`](./phase-A-documentation.md) | **Initialize documentation workspace — the first thing we do.** Scaffold the `docs/` template workspace; populate it with design-time content (artifact, open-questions, ADRs, specs as `Draft`, research, architecture, plans). No code written. Specs have unfilled `Verified by: [—]` — closing those is Phase 7's job | none |
| **B** | [`phase-B-bootstrap.md`](./phase-B-bootstrap.md) | Workspace scaffold, infra, CI, quarantine Python | Phase A done |
| **0** | [`phase-0-foundations.md`](./phase-0-foundations.md) | The irreversible core: `domain`, instrument metadata, strategy format freeze, raw archive design | Phase B done |
| **1** | [`phase-1-spine.md`](./phase-1-spine.md) | API + bus + one collector + storage writers + bar builder | Phase 0 done |
| **2** | [`phase-2-money-safety.md`](./phase-2-money-safety.md) | Risk gate + kill switch + paper execution + manual orders + reconciliation | Phase 1 done |
| **3** | [`phase-3-ui-streaming.md`](./phase-3-ui-streaming.md) | UI gateway + React panels + demand manager | Phase 2 done |
| **4** | [`phase-4-strategies.md`](./phase-4-strategies.md) | Feature engine + strategy runtime + backtest + multi-asset | Phase 2 done (UI helps but not required) |
| **5** | [`phase-5-front-doors.md`](./phase-5-front-doors.md) | JSON strategy API + visual builder + MCP server | Phase 4 done; strategy format frozen (Phase 0) |
| **6** | [`phase-6-second-asset.md`](./phase-6-second-asset.md) | Equity collector + equity broker adapter — proves the abstraction | Phase 4 done |
| **7** | [`phase-7-cutover.md`](./phase-7-cutover.md) | Parity verification, decommission `legacy_python/`, **finalize documentation workspace** (advance specs to `Implemented`, fill evidence, operational procedures from reality, full traceability verify), release | All prior phases done |

**Decision gates that block code (from [`../open-questions.md`](../open-questions.md)).**
These must be answered before the phase that depends on them; each phase file restates its gate:
- **Q1 real-vs-paper** — gates Phase 2 execution. **Resolved: Alpaca paper account for all paper trading (all assets, all domains).** Build the Alpaca adapter in Phase 2; Coinbase live adapter is post-Phase-6 scope.
- **Q2 broker/venue choice** — gates Phase 1 collector and Phase 6 equity adapter. **Resolved:** Three parallel systems, all assets/domains:
  - **Live execution:** Coinbase REST+WS (`crates/execution/src/coinbase.rs`)
  - **Paper execution:** Alpaca paper account (`crates/execution/src/alpaca.rs`)
  - **Backtest execution:** market_simulator — github.com/MHughesDev/market_simulator (`crates/execution/src/market_simulator.rs`)
  - **Crypto market data:** Kraken WS (`crates/collectors/src/crypto/kraken.rs`)
  - **Equity market data:** Alpaca data feed (`crates/collectors/src/equity/alpaca_data.rs`)
  - **Venue routing:** `crates/venue-router` resolves `(AssetClass, DataType)` → venue at runtime; data pipelines start **only on demand**, never on system init.
- **Q3 strategy-format 1.0 freeze** — gates Phase 5 (and is produced in Phase 0).
- **Q4 capital/liability** — gates any real-money switch (post-Phase 2, not in this plan's scope).

---

## 3.1 START HERE — execution entry point (read this before doing anything)

**To begin implementation, do exactly this:**

1. **Read this master plan** (you are here), then read the phase file you are about to execute.
2. **Execute phases strictly in order:** A → B → 0 → 1 → 2 → 3 → 4 → 5 → 6 → 7. Do not start a phase
   until the previous phase's **exit criteria** are all green. (Phase 4 may begin once Phase 2 is
   done even if Phase 3 is not — that is the only parallelism allowed; the phase file states it.)
3. **Execute the phase plans from `docs/plans/`** — those files. They are the
   canonical, self-contained, executable plans. Each references the stable spec at
   `[deleted - see Phase 7]/spec/` and the structure at `[deleted - see Phase 7]/file-structure.md`, both of
   which live in `[deleted - see Phase 7]/` and **persist unmodified until the very end** (see §2.9).
   So every reference in every phase file stays valid for the entire refactor.
4. **Within a phase, execute tasks by their dependency order** (each task lists `Depends on:`). A task
   is done only when its **acceptance criteria pass** — including the adversarial test where one is
   required (§2.8). "It compiles" is not "done."
5. **Track progress** by the task IDs (§7).

**Two copies of the plans — which is which (do not get confused):**
- `docs/plans/` = the **canonical executable plans**. Read and
  execute from here. Never edit the files under `[deleted - see Phase 7]/` (§2.9) — track progress
  externally (PR/issue checklist), not by editing these.
- `docs/plans/` (created by Phase A, task P A-T08) = a **traceable Formal-plan mirror** that is part
  of the `docs/` documentation deliverable (with **Derived From** headers linking to `docs/specs/`).
  It is the *record*, not the thing you execute from. If the two ever appear to disagree on technical
  content, the `[deleted - see Phase 7]/` copy wins.

**Canonical content reference:** Until Phase A runs, `[deleted - see Phase 7]/spec/` and
`[deleted - see Phase 7]/file-structure.md` are the only spec/structure docs. After Phase A,
`docs/specs/` and `docs/architecture.md` are the *restructured* canonical versions for the
documentation record — but the phase files keep citing the `[deleted - see Phase 7]/` paths (which still
exist) so you never need to re-resolve a reference mid-refactor. Both say the same thing; the `spec/`
wording is the source the `docs/specs/` versions were derived from.

---

## 4. How each phase file is written (so it is Haiku-4.5-ready)

**Every phase file is self-contained.** It assumes the executor has *only* that file, the spec
folder, and the file-structure doc — not this conversation and not memory of earlier phases beyond
what the file restates. This is deliberate: the plans must be executable by **Claude Haiku 4.5**
with full context, so the model can be switched at any time without losing the thread.

Each phase file contains, in this order:
1. **Phase goal** — one paragraph: what exists at the end that didn't before.
2. **Prerequisites** — which phases/crates must already exist; which decision gate must be answered.
3. **Invariants reminder** — the subset of §2 rules that this phase can violate if careless.
4. **Task list** — granular, block-by-block (not line-by-line). Each task has:
   - a stable **task ID** (e.g. `P1-T03`),
   - **goal**, **files to create/modify** (exact paths from `file-structure.md`),
   - **context** the executor needs (relevant types, spec section, the Python file to port behavior
     from if any),
   - **acceptance criteria** (including the adversarial test where §2.8 applies),
   - **dependencies** (other task IDs).
5. **Phase exit criteria** — the checklist that must be green to call the phase done.

A task is "block-by-block": e.g. "implement the bar builder + its watermark/revision logic + its
determinism test" is one task, not three hundred lines enumerated.

---

## 5. Migration mechanics (Python → Rust without losing behavior)

- **`[deleted - see Phase 7]/` is the permanent reference anchor — never touch it until the very end.**
  It lives at the repo root, read-only and unmodified, from Phase A through Phase 6. Every phase
  reads from it; no phase writes to it, moves it, or removes files from it. Deletion happens once in
  Phase 7 (P7-T07) as the final act after everything else is verified done.
- **Quarantine, don't delete (until Phase 7).** Phase B moves the entire current Python tree into
  `legacy_python/`. It stays as a behavior reference and is only deleted in Phase 7 after parity is
  verified. Do not import from it; read it to port behavior.
- **Port behavior, not structure.** When a Rust task says "port X from Python," it means replicate
  the *observable behavior and edge cases*, mapped onto the clean Rust structure — not transliterate
  the Python file layout.
- **Strangler pattern at the seam.** Until Phase 7, the Python system can keep running for reference;
  the Rust system is built and validated in parallel. There is no requirement to run both in prod
  simultaneously for this local-first scope, but the Python remains runnable for A/B behavior checks.
- **Frontend re-point, not rewrite.** `frontend/` stays; its `api/rest.ts` and `api/ws.ts` are
  re-pointed at the Rust endpoints as those endpoints land (Phases 1–5).

---

## 6. Definition of done for the whole refactor

- Every crate and file in `docs/architecture.md` (the as-built structural contract) exists and
  compiles.
- `just test` (workspace tests + integration tests) is green, including every adversarial test
  required by §2.8.
- The React frontend runs against the Rust backend: live panels stream, a user can click an asset
  and initialize a strategy on it (all three front doors), manual orders flow through the risk gate
  to paper execution, and a backtest can be submitted that delegates to `market_simulator` and
  returns results via the adapter.
- The equity asset class works through the **same** schema/metadata/risk path as crypto (Phase 6) —
  proving the abstraction.
- `legacy_python/` is deleted; no code references it.
- The `docs/` workspace is finalized: every spec `Implemented` with verified evidence, `architecture.md`
  reconciled to reality, operational procedures (`operate-the-stack`, `add-a-venue`) written from the
  actual system, `verify-traceability` all 10 steps green.
- **`[deleted - see Phase 7]/` is deleted from the repo root.** This is the last item. When this is
  done, the refactor is done.

---

## 7. Tracking

Each phase file's task IDs are the unit of tracking. Recommended: maintain a simple checklist (in
the PR description or an issue) of task IDs per phase, flipping them as the adversarial test for each
goes green. Do not mark a task done on "it compiles" — mark it done on "its acceptance criteria,
including the test, pass."
