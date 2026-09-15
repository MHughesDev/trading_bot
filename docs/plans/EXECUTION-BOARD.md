---
Type: Formal
Status: Current
Created: 2026-09-15
Derived From: docs/plans/ROADMAP.md, Set S MASTER §5, Set P MASTER §5, Set T MASTER §5,
  docs/procedures/execute-plan.md, ml-platform-context-pack/CLAUDE.md
---

# The Execution Board

**The coordination contract for running many implementation agents at once on one repository.**
[`ROADMAP.md`](./ROADMAP.md) says what to build and in what order. This file says who is
building what *right now*, what each track may touch, and what every track must not.

> Read this **before** your plan file, not after. A track that takes another track's migration
> number or edits another track's crate costs more to untangle than it saved.

---

## 1. Live tracks

Fifteen tracks, dispatched 2026-09-15. Each is one agent, one branch, one disjoint file set.

| # | Track | Plan | Branch | Migrations | Blocks |
|---|---|---|---|---|---|
| **1** | Set S Phase 0 → Phase 1 — ground rules, then contracts | [set-S/phase-0](plan-sets/set-S/phase-0-ground-rules.md), [phase-1](plan-sets/set-S/phase-1-contracts.md) | `claude/set-s-phase-0-ground-rules` | none, then 0061 if needed | **everything in Set S** |
| **2** | Set S Phase 7 — the monitor (COMP-007) | [set-S/phase-7](plan-sets/set-S/phase-7-monitor.md) | `claude/set-s-phase-7-monitor` | 0074–0078 | Set T Phase 2 |
| **3** | Set P Phase 0 — census + forward collectors | [set-P/phase-0](plan-sets/set-P/phase-0-measure-and-collect.md) | `claude/set-p-phase-0-measure` | 0097–0098 | Set P Phases 1–4 |
| **4** | Set P Phase 1 — deep backfill | [set-P/phase-1](plan-sets/set-P/phase-1-deep-backfill.md) | `claude/set-p-phase-1-backfill` | 0099 | Set P Phase 2 |
| **5** | Set T Phase 0 — reconciliation, Gate 15 | [set-T/phase-0](plan-sets/set-T/phase-0-reconciliation.md) | `claude/set-t-phase-0-reconciliation` | 0105–0106 | Set T Phases 1–3 |
| **6** | Pack §5.1 — AT-61 bit-identical resume | `ml-platform-context-pack/decisions/HANDOFF.md` §5.1 | `claude/pack-at61-resume` | none | every framework's resume claim |
| **7** | Pack §5.2 — campaign child workers `Study`, `EvalTask` | HANDOFF §5.2 | `claude/pack-campaign-workers` | 0113 | campaigns dispatching at all |
| **8** | Pack §5.4 — Phase 3 writers (`asset_embedding`, tensor) | HANDOFF §5.4 | `claude/pack-phase3-writers` | 0114 | the knowledge plane having any rows |
| **9** | Set P Phase 3 — alternative data (FRED/ALFRED, EDGAR, news, Kalshi) | [set-P/phase-3](plan-sets/set-P/phase-3-alternative-data.md) | `claude/set-p-phase-3-altdata` | 0102–0103 | Stage 5 regime work |
| **10** | AT-64 — the external MCP door's audit trail | HANDOFF §4 | `claude/pack-at64-mcp-audit` | 0116 | INV-16 having zero exemptions |
| **11** | Plan hygiene — audit Sets C, E, F, G against the code | ROADMAP §9 G-5, G-6 | `claude/plan-hygiene-audit` | none | Stage 3 (which Set E items block unattended paper) |
| **12** | `regime_scopes` configured + the OOS regime-conditional separation test | ROADMAP §9 G-1; SPEC §5.4 | `claude/regime-scopes-and-separation` | 0117 | **Gates 6 and 11**, which are dark today |
| **13** | The insight store — retrieval, 4K cap, decay-on-contradiction, local embedder | ROADMAP §9 G-1b; SPEC §5.6; checklist 3.10 | `claude/insight-store-retrieval` | 0118 | the knowledge base compounding |
| **14** | The gate stack's missing inputs — gates 3, 4, 10, 12 | HANDOFF §5.3; SPEC §12.3 | `claude/gate-evidence-sources` | 0119 | **Stage 2's exit — a complete verdict** |
| **15** | The simulator must honour declared exits (`ExitTracker`) | HANDOFF §5a | `claude/simulator-honours-exits` | 0120 if needed | every backtest of an exit-declaring strategy being correct |

Tracks 6, 7, 8, 10, 12, 13, 14 and 15 take migrations from the roadmap's Stage-2 reserve (0113–0120), **not**
from Set S's, Set P's or Set T's ranges. Track 11 writes no code.

**Track 11 is read-only against production code** and must not edit `ROADMAP.md`,
`EXECUTION-BOARD.md`, `artifact.md` or any `set-{P,S,T}` file — it reports what should change
in those rather than changing them.

`ResearchRun`, the third campaign child worker, is **not** in track 7 — it is Set S task S-10.6
and belongs to the lifecycle phase. Do not build it twice.

---

## 2. Rules that exist because fifteen agents share one workspace

### 2.1 Migrations

**Take numbers only from your own range.** Latest on disk is `0060_web_and_skills.sql`.
If your range is exhausted, stop and say so — do not borrow.

Migration checksums are byte-exact and CRLF breaks them. `.gitattributes` pins
`migrations/*.sql` to LF. Migrations are `include_str!`-embedded, so editing a `.sql` needs a
rebuild of `storage`.

### 2.2 Shared files — coordinate, never rebase blindly

`Cargo.toml` (workspace members) · `Cargo.lock` · `crates/mcp-server/src/taxonomy.rs` ·
`crates/api/src/routes/mod.rs` · `frontend/src/App.tsx` ·
`frontend/src/components/layout/TopBar.tsx` · `crates/collectors/src/lib.rs` ·
`crates/dataplane/src/source.rs` · `clickhouse/*.sql` · `crates/reconciliation/src/lib.rs` ·
`crates/execution/src/lib.rs`

Touch one of these only for the minimum your track needs — one line added, never a
reorganisation. If your change to a shared file is more than a few lines, say so in your report
rather than landing it quietly.

### 2.3 The build box cannot run fifteen builds

Measured on this machine, and none of it is negotiable:

- **`.cargo/config.toml` pins `jobs = 2`. Never raise it.** Raising `-j` reproducibly OOMs the
  Windows page file and surfaces as misleading `E0463 can't find crate` / `E0786 invalid
  metadata` cascades that look like source errors.
- **Never run `cargo test --workspace` or `cargo build --workspace`.** Use `-p <your crate>`.
  Fifteen agents running workspace builds will fill the disk and thrash the page file.
- **`target/` is ~237 GB.** Check free space before anything large. `cargo clean` costs a
  rebuild of 639 crates. **Never delete a Docker VHDX to make room.**
- **Never pipe cargo through another command** — it hides the exit code.
- `xtask` scans `git ls-files`, so `check-money-f64` and `check-bars-v1-frozen` skip files you
  have not `git add`ed. **`git add` before you trust a green run.**

### 2.4 Do not touch these files — they are the user's

From `BS-007/HANDOFF.MD` §6 and the working tree at dispatch:
`Cargo.lock`, `Cargo.toml`, `apps/platform/src/main.rs`, `crates/config/Cargo.toml`,
`crates/config/src/lib.rs`.

If your track genuinely needs a workspace member added to `Cargo.toml`, **stop and report it**
rather than committing it. This is roadmap gate **H-1** and it is being resolved by track 1.

### 2.5 Pre-existing failures — record, do not chase

Four `storage` live suites (`ledger_append`, `ledger_writer`, `pnl_schema` want a
`ledger_events` table no migration creates; `registry_seed` expects 8 asset classes where 0006
seeds 6) and `cargo xtask lint-no-json-hotpath` (one pre-existing line in
`crates/collectors/src/crypto/kraken.rs`). All were failing before this work. **Do not fix them
into your PR.**

---

## 3. Definition of done — every task, every track

A task is done when **all five** are true. Four out of five is not done.

1. The behaviour matches the spec section the task traces to.
2. The spec's acceptance criterion has its `Verified by:` filled with the test name.
3. **Every decided mechanism has an adversarial test proving the violation is *rejected*** —
   not only that the happy path works. A test that asserts the feature works is half a test.
4. No invariant regressed.
5. A judgment call, if any, is recorded in
   `docs/ml-platform-context-pack/decisions/ADR-INDEX.md` (check it for the next id — it is not
   in numeric order).

**And the rule that outranks all five:** never report a partially enforced invariant as
enforced. Partial is a valid, respected answer. Overstated is not.

---

## 4. Non-negotiables, inherited

These hold in every track, in tests too:

- **No non-PIT read path.** Ever. `BarStore` is the single reader.
- **No way to run a trial without a ledger row** (INV-16), and no flag that disables either.
- **No f64 on a price or size** (SC-1) — `cargo xtask check-money-f64`.
- **No second feature implementation** (INV-14/AT-15) — extend `crates/features`, never add a
  parallel engine.
- **No live-venue code path** from anything in Set T. Paper only.
- **Authority order:** `INVARIANTS.md` > `spec/SPEC.md` > reference > your judgment.

---

## 5. When you get stuck

| Situation | Mark it | Then |
|---|---|---|
| The spec is wrong | `BLOCKED:UPSTREAM` | write the correction into the spec with a dated note, set plans citing it `Derivation Status: STALE`, re-derive, resume |
| A source's terms forbid what the spec assumed | `BLOCKED:UPSTREAM` | quote the term in the task, name the next candidate, move on — never build against terms |
| It needs a human decision or a credential | `BLOCKED:HUMAN` | name the roadmap gate (§8 of ROADMAP.md), move to your next task, do not guess |
| Another track owns the file you need | — | report it; do not edit across tracks |

A deliberate deviation that is *not* a spec error is declared in the task in the Set L form:
"**T-0.2 done — as a dual-write, which is a deliberate deviation from this task as written**",
with the reason.

---

## 6. Reporting

At the end of your run, report exactly this, in this order:

1. **Tasks completed**, by id, each with the test name that proves it.
2. **Tasks partially done**, by id, with precisely what is missing.
3. **Tasks blocked**, by id, with the mark from §5 and what would unblock them.
4. **Shared files touched**, and what changed in each.
5. **Anything you found that is wrong with the plan or the spec** — this is the most valuable
   thing you can return, and it is not a criticism of anyone.
6. **Pre-existing failures you observed**, so the list in §2.5 stays current.

Update your phase file's completion line and status labels as you go, not at the end.

---

## 7. The queue — what gets dispatched when a track lands

**No track finishing should leave the board idle.** When a track reports, dispatch its
successors immediately. Ordered by what each unblocks.

| When this lands | Dispatch | Why it was waiting |
|---|---|---|
| **Track 1** (Set S Phase 1, contracts) | **Set S Phases 2, 3, 4, 5, 6 — five tracks at once** | all five need the one `ToolSpec`, `WorkingState` and the generated taxonomy. This is the single biggest unlock on the board |
| Track 3 (census + forward collectors) | Set P Phase 2 (quality and eligibility) | `data_qc` needs the census's coverage inputs |
| Track 4 (deep backfill) | Set P Phase 2, then Phase 4 | nothing to grade or resample without history |
| Track 5 (reconciliation, Gate 15) | Set T Phase 1 (the deployment path) | one door into ring 2, with a human at it |
| Tracks 2 **and** 5 | Set T Phase 2 (the paper fleet) | the fleet cannot be drilled until the monitor runs |
| Track 11 (plan hygiene) | a re-cut Set E track, scoped to **only** the items that block unattended paper trading | Stage 3 needs that subset, not all 30 |
| Tracks 5 **and** 14 | nothing new — **check whether all sixteen gates now compute.** That is Stage 2's exit | a complete verdict is the milestone, not the tasks |
| Set S Phases 2 + 4 + 8 | Set S Phases 9, 10, 11 (retrieval, lifecycle, continuous research) | the lifecycle needs the effect log, memory and the workflows |

**Not yet dispatchable, and why** — do not spawn these on a quiet moment:

- **Set M-residual** and **Set N** (Stage 4) — blocked on roadmap gaps **G-3** and **G-4**:
  FEAT-005 predates the ledger and the fee book, and Set H Phases 4–5 were written against the
  pre-`mlops` registry. Both need a spec reconciliation first. Planning from a stale spec is
  the failure the roadmap exists to prevent.
- **Set R-a** (skills lifecycle) — blocked on **G-2**; AGENT-003 is a Phase-0 contract, not a
  build spec.
- **Stage 6 / anything touching capital** — blocked on **H-4**, which is a conversation about
  liability, not a task.

---

## 8. Verifying the board

Before you report, and after any edit to a plan file:

```bash
python scripts/verify_plans.py
```

It checks every plan set in one pass: per-phase task counts against the header, the
`**Acceptance:**`/`**Traces:**`/status-label counts against the task count, each MASTER's total
against its phase files and its own phase table, every relative link, task-id uniqueness, and —
the one that matters most with fifteen agents live — **migration-range collisions between sets
and against what is already on disk**.

**It was green when the board was created** (236 planned tasks across Sets P, S and T; highest
migration on disk 0060; no collisions). A green baseline is the point: if it fails after your
work, you broke something, and the output names the file. Do not add an exception to make it
pass — fix the file, or if the check itself is wrong, say so in your report.

---

## 9. Board log

| Date | Change |
|---|---|
| 2026-09-15 | Track 15: the simulator does not call `ExitTracker`, so exits declared at `definition_version` 1.3 are ignored in every backtest. Found while reconciling FEAT-005 for Stage 4; dispatched ahead of that work. Fifteen tracks live. |
| 2026-09-15 | Track 14: the gate stack's missing inputs (gates 3, 4, 10, 12). With track 5's Gate 15 that is the whole of the five dark gates, and Stage 2's exit criterion. §7 (the dispatch queue) added so no landing leaves the board idle. Fourteen tracks live. |
| 2026-09-15 | Third wave: tracks 12 (`regime_scopes` + the separation test SPEC §5.4 calls the difference between a real regime model and a decorative one) and 13 (the insight store's read path and decay). Both came from correcting ROADMAP §9 G-1, which wrongly recorded the regime engine as unspecified — it is specified and its rule tier runs; what was missing was configuration and validation. Thirteen tracks live. |
| 2026-09-15 | Second wave: tracks 9 (Set P Phase 3 alt data), 10 (AT-64 external MCP audit trail — the last INV-16 exemption), 11 (plan hygiene audit of Sets C, E, F, G). Eleven tracks live. |
| 2026-09-15 | Board created. Eight tracks dispatched: Set S Phases 0→1 and 7, Set P Phases 0 and 1, Set T Phase 0, and pack items §5.1, §5.2, §5.4. Migration reserve 0113–0120 opened for the pack tracks. |
