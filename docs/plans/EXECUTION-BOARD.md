---
Type: Formal
Status: Superseded
Superseded By: docs/plans/PLAN.md §3a
Superseded On: 2026-09-27
Created: 2026-09-15
Derived From: docs/plans/ROADMAP.md, Set S MASTER §5, Set P MASTER §5, Set T MASTER §5,
  docs/procedures/execute-plan.md, ml-platform-context-pack/CLAUDE.md
---

# The Execution Board

> ## SUPERSEDED, 2026-09-27. Read [PLAN.md §3a](./PLAN.md) instead.
>
> **This file's coordination model no longer applies, and following it would now
> cause the harm it was written to prevent.** It partitions agents by *branch and
> worktree*, where the hazard is the merge. Every agent since has run in the
> single main tree — a standing instruction — where there is no merge and the
> hazard is the opposite one: two agents editing the same file, and one agent's
> half-written file breaking another's build. The partition that works there is
> by **file territory**, which is what PLAN.md §3a specifies.
>
> **§1's track table is history, not state.** Its eighteen tracks were dispatched
> on 2026-09-15 and are named by branches that have since either landed, been
> salvaged into main, or gone stale — `claude/set-p-phase-3-altdata` is 272
> commits behind main and its collectors reached main by a different route
> entirely. Nothing reads this table to decide anything, and a reader who took it
> for current would look for work in places it is no longer being done.
>
> Kept rather than deleted because the reasoning in §2 onward — migration-number
> allocation, the shared-file list, what a track must never touch — is what
> PLAN.md §3a was derived *from*, and the record of how this programme learned to
> run agents in parallel is worth more than the disk it costs. Do not update the
> track table; it is dated on purpose.

**The coordination contract for running many implementation agents at once on one repository.**
[`ROADMAP.md`](./ROADMAP.md) says what to build and in what order. This file said who was
building what at the time it was written, what each track could touch, and what every track
must not.

---

## 1. Live tracks

Eighteen tracks dispatched (two landed), 2026-09-15. Each is one agent, one branch, one disjoint file set.
Fifteen tracks, dispatched 2026-09-15. Each is one agent, one branch, one disjoint file set.

| # | Track | Plan | Branch | Migrations | Blocks |
|---|---|---|---|---|---|
| ~~1~~ | Set S Phases 0→1 — ground rules, then contracts | [set-S/phase-0](plan-sets/set-S/phase-0-ground-rules.md), [phase-1](plan-sets/set-S/phase-1-contracts.md) | `claude/set-s-phase-0-ground-rules` | none | **LANDED 2026-09-15** — the three contracts exist (`ToolSpec`, generated taxonomy, `WorkingState`); S-0.1 `BLOCKED:HUMAN` on H-1; `Loop` does not yet own `WorkingState` (track 21) |
| **2** | Set S Phase 7 — the monitor (COMP-007) | [set-S/phase-7](plan-sets/set-S/phase-7-monitor.md) | `claude/set-s-phase-7-monitor` | 0074–0078 | Set T Phase 2 |
| ~~3~~ | Set P Phase 0 — census + forward collectors | [set-P/phase-0](plan-sets/set-P/phase-0-measure-and-collect.md) | `claude/set-p-phase-0-measure` | 0097–0098 | **LANDED 2026-09-15** — census published; Kraken Futures collecting; Deribit `BLOCKED:UPSTREAM` on ToS; P-0.2 not started, P-0.6 partial |
| **4** | Set P Phase 1 — deep backfill | [set-P/phase-1](plan-sets/set-P/phase-1-deep-backfill.md) | `claude/set-p-phase-1-backfill` | 0099 | Set P Phase 2 |
| ~~5~~ | Set T Phase 0 — reconciliation, Gate 15 | [set-T/phase-0](plan-sets/set-T/phase-0-reconciliation.md) | `claude/set-t-phase-0-reconciliation` | 0105–0106 | **LANDED 2026-09-15** — T-0.1/0.2/0.3 done (T-0.3's UI surface deliberately not built); T-0.4 blocked on track 16 **and** on Alpaca credentials |
| ~~6~~ | Pack §5.1 — AT-61 bit-identical resume | HANDOFF §5.1 | `claude/pack-at61-resume` | none | **LANDED 2026-09-15** — torch proved bit-identical; GBDTs only with sampling off; found the two real blockers nobody had listed |
| **7** | Pack §5.2 — campaign child workers `Study`, `EvalTask` | HANDOFF §5.2 | `claude/pack-campaign-workers` | 0113 | campaigns dispatching at all |
| **8** | Pack §5.4 — Phase 3 writers (`asset_embedding`, tensor) | HANDOFF §5.4 | `claude/pack-phase3-writers` | 0114 | the knowledge plane having any rows |
| **9** | Set P Phase 3 — alternative data (FRED/ALFRED, EDGAR, news, Kalshi) | [set-P/phase-3](plan-sets/set-P/phase-3-alternative-data.md) | `claude/set-p-phase-3-altdata` | 0102–0103 | Stage 5 regime work |
| **10** | AT-64 — the external MCP door's audit trail | HANDOFF §4 | `claude/pack-at64-mcp-audit` | 0116 | **DONE-5** — the last INV-16 exemption |
| ~~11~~ | Plan hygiene — audit Sets C, E, F, G | ROADMAP §9 G-5, G-6 | `claude/plan-hygiene-audit` | none | **LANDED 2026-09-15** — closed G-5, G-6, H-5; found the execution path is a facade |
| **12** | `regime_scopes` configured + the OOS regime-conditional separation test | ROADMAP §9 G-1; SPEC §5.4 | `claude/regime-scopes-and-separation` | 0117 | **Gates 6 and 11** — with track 14, the funnel reaches 16/16 |
| **13** | The insight store — retrieval, 4K cap, decay-on-contradiction, local embedder | ROADMAP §9 G-1b; SPEC §5.6; checklist 3.10 | `claude/insight-store-retrieval` | 0118 | the knowledge base compounding |
| **14** | The gate stack's missing inputs — gates 3, 4, 10, 12 | HANDOFF §5.3; SPEC §12.3 | `claude/gate-evidence-sources` | 0119 | **Stage 2's exit — a complete verdict** |
| ~~15~~ | The simulator must honour declared exits | HANDOFF §5a | `claude/simulator-honours-exits` | 0120 | **LANDED 2026-09-15** — 11 hand-computed tests; also found zero-commission and zero-size on *every* run, and that exits still do not fire live |
| **16** | **The execution path to a fill** — stage 3, sizing, exits-on-save, the real risk context, order/fill persistence | Set E 5.4/5.1/4.4/4.2/2.3; COMP-002 | `claude/execution-path-to-a-fill` | 0121 | **DONE-3 — the critical path for the whole programme** |
| **17** | Streaming and lifecycle robustness — WS fan-out, ring-consumer spin, graceful shutdown, the JSON hot-path lint into CI | Set C #14/#15 + 2 unfiled | `claude/streaming-and-shutdown` | none | durable writes surviving an abrupt stop |
| **18** | Resume's producer and consumer — an adapter that checkpoints mid-run, and a caller for `plan_resume` | track 6's finding; `crates/jobs/src/checkpoint.rs` | `claude/resume-producer-and-consumer` | 0122 | a killed training job continuing instead of restarting |
| **19** | The **live** runtime must honour exits too — SC-3 | track 15's closing finding; `artifact.md` SC-3 | `claude/live-runtime-honours-exits` | 0123 if needed | a declared stop-loss bounding a deployment, not just a backtest |
| **20** | Repair `knowledge_time` provenance — every existing row claims to have been observed | census P-0.1; ADR-P2-24; INV-01 | `claude/knowledge-time-provenance-repair` | 0124 | **any bar being Gate-15 admissible** |
| **21** | Set S Phase 2 — one runtime loop; `Loop` takes ownership of `WorkingState` | [set-S/phase-2](plan-sets/set-S/phase-2-one-runtime.md) | `claude/set-s-phase-2-one-runtime` | 0061–0064 | Set S Phases 8, 9, 10, 11 |
| **22** | Set S Phase 4 — memory and evaluation | [set-S/phase-4](plan-sets/set-S/phase-4-memory-and-evaluation.md) | `claude/set-s-phase-4-memory-evals` | 0065–0073 | **every other phase's exit gate** |
| **23** | Set S Phase 3 — the catalogue (~26 tool additions) | [set-S/phase-3](plan-sets/set-S/phase-3-catalogue.md) | `claude/set-s-phase-3-catalogue` | 0079–0081 | Set S Phase 8 |
| **24** | Set S Phase 5 — the strategy language (DATA-006 engine, SLv2, `preview_signals`) | [set-S/phase-5](plan-sets/set-S/phase-5-strategy-language.md) | `claude/set-s-phase-5-language` | 0082–0084 | **the archetype ceiling** — today it is `ema_N`/`rsi_N` |
| **25** | Set S Phase 6 — the Research Desk (Chat/Research + Continuous toggle) | [set-S/phase-6](plan-sets/set-S/phase-6-research-desk.md) | `claude/set-s-phase-6-desk` | 0125 if needed | **DONE-1 — a question typed into the desk** |
| **26** | Set S Phase 1 remainder — the context budget (S-1.2, 1.5–1.9, 1.11, 1.12) | [set-S/phase-1](plan-sets/set-S/phase-1-contracts.md); AGENT-005 Part I | `claude/set-s-phase-1-context-budget` | 0126 if needed | **Phase 9** — Part II may only ship by beating Part I |
| **25** | Set S Phase 6 — the Research Desk (AGENT-012) | [set-S/phase-6](plan-sets/set-S/phase-6-research-desk.md) | `claude/set-s-phase-6-desk` | none (0125 reserved, unused) | nothing |

**H-1 closed 2026-09-15 (`37faade`) — every paused and running track should rebase onto
`main` before doing anything else. See §2.5a.**

**State as of the 2026-09-15 rate-limit incident.** Two tracks landed (6, 11). Five are
**resumed and running**: 16, 3, 1, 15, 5. Eleven are **paused with their worktrees intact** and
are resumed with `SendMessage`, never re-dispatched — re-dispatching discards their work:

| Paused | Was mid- |
|---|---|
| 2 monitor | mapping a new rejection in the orders route |
| 4 backfill | the `BarSink` impl for `BarStore` plus the live revision test |
| 7 campaign workers | unit tests for the termination rules |
| 8 knowledge writers | registering modules and one line in `routes/mod.rs` |
| 9 alt data | the Kalshi exposure-only test |
| 10 MCP audit | the adversarial tests |
| 12 regime | wiring the regime job to the registry, plus the separation job |
| 13 insight store | the HTTP route module |
| 14 gate inputs | the Postgres implementation |
| 17 streaming | the test lock in `live_bus.rs` |
| 18 resume | budget enforcement in the worker (AT-61 still passing at 78 tests) |

Three worktrees are on a `worktree-agent-*` branch rather than their intended `claude/*` one
(tracks 5, 12, 14) — note it at integration so their work is found.

Tracks 6, 7, 8, 10, 12, 13, 14 and 15 take migrations from the roadmap's Stage-2 reserve (0113–0120), **not**
from Set S's, Set P's or Set T's ranges. Track 11 writes no code.

**Track 11 is read-only against production code** and must not edit `ROADMAP.md`,
`EXECUTION-BOARD.md`, `artifact.md` or any `set-{P,S,T}` file — it reports what should change
in those rather than changing them.

Track 25 writes only `frontend/src/{api,hooks,lib,components/desk,pages}` plus the three shared
frontend files named in §2.2 (`App.tsx`, `TopBar.tsx`, `AppLayout.tsx`), and deletes the two
surfaces the desk replaces (`/agent`, `/research`). It adds no migration and no Rust route.

`ResearchRun`, the third campaign child worker, is **not** in track 7 — it is Set S task S-10.6
and belongs to the lifecycle phase. Do not build it twice.

---

## 2. Rules that exist because eighteen agents share one workspace
## 2. Rules that exist because fifteen agents share one workspace

### 2.1 Migrations

**Take numbers only from your own range.** Latest on disk is `0060_web_and_skills.sql`.
If your range is exhausted, stop and say so — do not borrow.

Migration checksums are byte-exact and CRLF breaks them.

> **Corrected 2026-09-15 — this section was wrong, and wrong in the dangerous direction.**
> It said `.gitattributes` pins `migrations/*.sql` to LF. **`.gitattributes` does not exist in
> this repository**, and `core.autocrlf=true`, so every existing migration is **CRLF in the
> worktree**. The hazard the board told you was handled is live. Write your own migrations
> **LF**, expect a sqlx-checksum mismatch against an existing database, and repair only with
> `cargo run -p storage --example normalize_checksums` (it refuses anything that is not
> provably a line-ending difference).
>
> **Do not create `.gitattributes` or renormalise the repo yourself.** Doing that while twenty
> branches are in flight would rewrite line endings under all of them. It is a decision for the
> user, at integration.

Migrations are `include_str!`-embedded, so editing a `.sql` needs a rebuild of `storage`.

### 2.2 Shared files — coordinate, never rebase blindly

`Cargo.toml` (workspace members) · `Cargo.lock` · `crates/mcp-server/src/taxonomy.rs` ·
`crates/api/src/routes/mod.rs` · `frontend/src/App.tsx` ·
`frontend/src/components/layout/TopBar.tsx` · `crates/collectors/src/lib.rs` ·
`crates/dataplane/src/source.rs` · `clickhouse/*.sql` · `crates/reconciliation/src/lib.rs` ·
`crates/execution/src/lib.rs` · **`.github/workflows/ci.yml`** (track 6 added a `resume-bit-identity`
job — append, never reorganise) · **`docs/ml-platform-context-pack/decisions/ADR-INDEX.md`**
(several tracks take ids concurrently; take the next free one you can see and expect a trivial
renumber at integration — do not block on it)

Touch one of these only for the minimum your track needs — one line added, never a
reorganisation. If your change to a shared file is more than a few lines, say so in your report
rather than landing it quietly.

### 2.3 The build box cannot run eighteen builds
### 2.3 The build box cannot run fifteen builds

Measured on this machine, and none of it is negotiable:

> ### ⚠ Corrected 2026-09-15 — **never share `CARGO_TARGET_DIR` between worktrees**
>
> Earlier briefs told every track to `export CARGO_TARGET_DIR=<repo>/target` so twenty agents
> would not create twenty copies of a 237 GB build directory. **That instruction was wrong and
> it corrupts builds.** Track 5 demonstrated it: identically-named workspace crates at identical
> *workspace-relative* paths produce colliding artifact hashes, so worktrees overwrite each
> other's `.rmeta`. It surfaces as a phantom `E0599 no variant named X` in a file that plainly
> contains `X`. In one observed run `cargo test -p domain` passed and `cargo test -p risk`
> immediately afterwards failed against a **stale `domain`**.
>
> **A stale artifact can make a test pass that should fail.** On a platform whose entire claim
> is honest evidence, a build system that silently lies is worse than a slow one. Correctness
> beats disk here, and it is not close.
>
> **Use your own worktree's `target/`.** Build narrowly (`-p <crate>`, never `--workspace`).
> `cargo clean -p <crate>` clears a contaminated artifact if you hit one.
>
> **And the disk argument that motivated the shared cache does not survive measurement.**
> Track 1 cold-built `mcp-server-lib`, `harness`, `ledger` and `invariants` with their **full**
> graphs — sqlx, clickhouse, storage, ed25519-dalek — and its private `target/` came to
> **4.5 GB**, with **217.8 GB still free**. Eighteen private directories at that size are about
> 80 GB. The repo-root `target/`'s 237 GB is *accumulated history across every build ever run
> here*, not one track's cost, and the cargo registry and git checkouts are paid once per
> machine rather than once per worktree. So the instruction traded a real correctness hazard for
> a saving that was never there (ADR-S1-07). Still check free space before a first build, still
> never delete a Docker VHDX.
>
> Note this very likely explains some of the `E0463`/`E0786` cascades the next bullet attributes
> to `-j`. **Both are real; do not raise `-j` either.**

- **`.cargo/config.toml` pins `jobs = 2`. Never raise it.** Raising `-j` reproducibly OOMs the
  Windows page file and surfaces as misleading `E0463 can't find crate` / `E0786 invalid
  metadata` cascades that look like source errors.
- **Never run `cargo test --workspace` or `cargo build --workspace`.** Use `-p <your crate>`.
  This matters twice over now that each worktree has its own target directory: narrow builds are
  what keep twenty of them from filling the disk.
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

### 2.5 Capacity — how many agents this account can actually run

**Measured the hard way on 2026-09-15: sixteen concurrent Opus agents exhausted the session
rate limit and every one of them was killed mid-task.** Not the build box — the API quota.
Two tracks had landed; the other sixteen stopped wherever they were.

What that cost, and what it did not:

- **Nothing was lost.** Every worktree survived with its work intact, mostly *uncommitted* —
  one track had 218 uncommitted paths. Agents are resumed with `SendMessage`, which restores
  their context, rather than re-dispatched, which would discard it.
- **Uncommitted work is the real exposure.** A killed agent's files are only as safe as the
  filesystem. **Commit per coherent piece, not once at the end** — this is now a standing
  instruction in every track brief, and it exists because of this incident.

**The rule going forward: resume or dispatch in waves of about five, prioritised by the
roadmap, and let landings pull the next wave in.** Sixteen at once is not faster than five at
a time if all sixteen die at hour three. The dispatch queue in §7 is what makes waves work —
it says what comes next without anyone having to re-derive it.

When choosing a wave, take tracks in this order: the critical path to DONE-3 first (track 16,
then 5), then anything irreversible (track 3's forward collectors), then the biggest unlock
(track 1's contracts), then live correctness defects (track 15), then everything else.

### 2.6 Pre-existing failures — record, do not chase

**Corrected 2026-09-15 — this list was incomplete and the omissions matter.** Failing before
any of this work:

- Four `storage` live suites — `ledger_append`, `ledger_writer`, `pnl_schema` want a
  `ledger_events` table no migration creates; `registry_seed` expects 8 asset classes where
  0006 seeds 6.
- `cargo xtask lint-no-json-hotpath` — one line in `crates/collectors/src/crypto/kraken.rs`.
- **`cargo xtask check-money-f64` fails.** Track 5 enumerated the 10 on a clean baseline:
  `apps/platform/src/book_feed.rs:72`; `backtest/src/gates/evaluators.rs:615,987`;
  `backtest/src/gates/profile.rs:70,72`; `backtest/src/synthetic.rs:153,204`;
  `dataplane/src/options.rs:60`; `ledger/src/neff.rs:198`; `ledger/src/outcome.rs:29`.
  Another track counted 16 across 8 files — the difference is what each worktree had `git
  add`ed, since `xtask` scans `git ls-files`. It was *not* on this list before and agents were
  told money-f64 was clean. **Do not add a violation of your own**: whether your diff adds a hit
  is a different question from whether the command exits zero.
- **`cargo test -p invariants --test static_invariants` — `spec_15_tool_surface_conformance`**
  ("promotion must fall under an approval envelope"), last touched by `ef35299`.
- **`cargo fmt --all -- --check` fails repo-wide at HEAD** — hundreds of files, because
  `rustfmt.toml` sets nightly-only options that silently do not apply. It is in `just`. **Format
  only your own files**; do not reformat the repo while twenty branches are in flight.

**Do not fix any of these into your PR.** Do check that you have not added to them.

### 2.5a The worktree baseline — **resolved 2026-09-15**

**Rebase onto `main`.** H-1 is closed at commit `37faade` (161 files, +27,518), and the
baseline every worktree was cut from is no longer the right one.

`main` now contains `ExitTracker`, `FeeBook`, the falsification ledger, `webfetch`,
`skills`, the evidence tools, migrations 0058–0060, `.gitattributes`, the two `Cargo.toml`
workspace-member lines, the 13 catalogue tool files — **and this board, the roadmap and
every plan-set file**, which were untracked as well.

**If you mirrored the user's tree into your branch, drop that commit rather than keeping
it.** The real commit supersedes it, and two copies of the same 27,000 lines will conflict
with each other at integration in a way that is much harder to read than a clean rebase.
Say in your report that you dropped it, so integration knows not to look for it.

Two warts to expect, neither of them yours to fix:

- **A stale `ledger` rlib** can make `cargo build -p api` fail with unresolved imports for
  modules that are declared and present. `touch crates/ledger/src/lib.rs` and rebuild. It is
  not a real breakage and it cost fifteen minutes to diagnose once already.
- **`docs/specs/README.MD` shows permanently dirty.** Both case variants are indexed and
  Windows holds one file. The content is committed under `README.md`; the `.MD` entry is
  stale. Do not `git add` it and do not try to remove it — §2.6 applies.

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

**One correction, 2026-09-15.** Earlier briefs told tracks "the oracle test must still pass"
as though it guarded the fill path. It does not — it is pure arithmetic in `costs.rs` and
never runs the simulator, so it guards the cost *algebra* only. It would not have caught the
zero-commission defect and will not catch a fill-timing regression. Keep it passing, but do
not treat it as cover for a change to how or when a fill happens; that needs its own test.

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
| ~~Track 1~~ **landed** | **All five phases dispatched — tracks 21, 22, 23, 24, 25.** The contracts exist and the whole Set S fan-out is live | — |
| **Phase 1 is not finished** — track 1 reached 5 of 20 tasks | **track 26 dispatched** for the context-budget half (S-1.2, 1.5–1.9, 1.11, 1.12). **Still unclaimed: S-1.14–S-1.20**, the reasoning-protocol half (AGENT-008) — plans made of tools, decisions remembered, goal classification, the decode ladder, budget correctness, the error contract, liveness | Phases 8 and 9 both need Phase 1 complete; dispatch the reasoning half on the next landing |
| Track 3 (census + forward collectors) | Set P Phase 2 (quality and eligibility) | `data_qc` needs the census's coverage inputs |
| Track 4 (deep backfill) | Set P Phase 2, then Phase 4 | nothing to grade or resample without history |
| Track 5 (reconciliation, Gate 15) | Set T Phase 1 (the deployment path) | one door into ring 2, with a human at it |
| Tracks 2 **and** 5 | Set T Phase 2 (the paper fleet) | the fleet cannot be drilled until the monitor runs |
| ~~Track 11~~ **landed** | **track 16, dispatched** — but scoped to the 4 items that block a fill *at all*, which turned out to be Stage 0 work, not the Stage 3 subset expected | the audit found the execution path is a facade; the remaining 6 unattended-paper blockers stay queued behind track 16 |
| ~~Track 6~~ **landed** | **track 18, dispatched** — the producer and consumer halves AT-61 revealed were missing | the handoff listed only the measurement as blocking; it was necessary but nowhere near sufficient |
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

## 8. Integration — the merge plan

Every track works in its own worktree on its own branch. **Nothing has been merged to `main`,
and nobody should merge without the user asking.** But the *order* is not a judgment call any
more — it falls out of the dependencies, and leaving it underived is its own risk. This section
is the plan, ready to execute when the user says go.

### 8.1 Order, and why

Merge in these waves. Within a wave, order does not matter.

| Wave | Branches | Why here |
|---|---|---|
| **0** | `claude/plan-hygiene-audit` | docs only; touches nothing any other branch compiles against |
| **1** | `claude/set-s-phase-0-ground-rules` (track 1) | **first among code.** It regenerates `taxonomy.rs` from `ToolSpec` and lands `WorkingState`. Five Set S phases build on it; merging it after them means rebasing all five |
| **2** | `claude/simulator-honours-exits` (15) | self-contained in `crates/backtest`; track 19 builds directly on its `ExitTracker` wiring |
| **3** | `claude/execution-path-to-a-fill` (16) | rewrites the order path — `hot_path.rs`, `orders.rs`, `risk/gate.rs`, order/fill persistence. Tracks 5, 2 and 17 all assume its shape |
| **4** | `claude/set-t-phase-0-reconciliation` (5), `claude/set-s-phase-7-monitor` (2), `claude/streaming-and-shutdown` (17) | all three touch what wave 3 rewrote. Track 5's `risk/gate.rs` change is +25 purely additive lines, so it rebases cleanly onto 16 rather than the reverse |
| **5** | `claude/set-p-phase-0-measure` (3), then `claude/set-p-phase-1-backfill` (4), then `claude/knowledge-time-provenance-repair` (20) | strictly ordered: 4 needs 3's `backfill_state`; 20 rewrites the same rows 4 writes |
| **6** | `claude/live-runtime-honours-exits` (19), `claude/set-p-phase-3-altdata` (9) | 19 needs wave 2; 9 needs wave 5's collector scaffolding |
| **7** | Set S phases: 21, 22, 23, 24, 25 | all need wave 1. Mutually disjoint by crate |
| **8** | pack tracks: 6, 7, 8, 10, 12, 13, 14, 18 | mostly disjoint. 18 needs 6; 14 shares `crates/backtest/src/gates/` with 5, so after wave 4 |
| **—** | ~~`claude/set-p-phase-2-quality`~~ | **already on `main` at `70cfc1f`.** Dispatched and landed after this table was written. Disjoint: a new `crates/dataplane/src/qc/` and migration 0100, which no branch claims. Its only shared touches are one `pub mod` line and a `toml` dependency |

### 8.2 Known conflicts, named in advance

- **`crates/mcp-server/src/taxonomy.rs`** — track 1 *regenerates* it from `ToolSpec` while other
  tracks may have added rows by hand. **Take track 1's generated version and re-derive**; a
  hand-added row that the generator does not produce is a row that should not exist.
- **`crates/risk/src/gate.rs`** — tracks 16, 5 and 2. Track 5's change is additive and documented
  (a `halts` field, `halt_registry()`, one `admit()` call, consulted *ahead of* the idempotency
  cache — preserve that ordering, it is load-bearing).
- **`crates/backtest/src/gates/`** — tracks 5 (Gate 15) and 14 (gates 3, 4, 10, 12). Disjoint
  gates, same directory.
- **`crates/jobs`** — tracks 8 and 18. **`crates/collectors/src/lib.rs`** — tracks 3, 4, 9.
- ~~**`docs/plans/plan-sets/set-T/phase-0-reconciliation.md`**~~ — **resolved 2026-09-15.** The
  H-1 commit swept in the shared checkout's **stale** copy (101 lines, zero `Verified by:`) over
  track 5's real one (290 lines, three). Track 5's has been taken into `main`. Two further
  sweeps of the same kind were found and corrected at the same time: `set-C/MASTER.md` (track
  11's audit, 402 lines over 264) and `set-P/P-0.4-BLOCKED-crypto-option-chains.md`, which the
  H-1 commit missed entirely. **The general lesson for integration: a bulk commit of an
  untracked tree takes the shared checkout's copy of every file, and for plan files the
  worktree's copy is usually the newer one.** Check line counts against the landed branches
  before assuming `main` is current.
- **`ADR-INDEX.md` ids have already collided.** Track 6 took `ADR-P6-11`; track 15 reports
  `ADR-P6-11`, `P6-12` and `P6-13`. Renumber at integration — the *content* is what matters and
  no decision is lost. Expect more of these; it was the accepted cost of not serialising.
- **Mirrored "user's work" commits.** Tracks 15 and 19 mirrored the user's uncommitted checkout
  into their branches to get code their tasks depended on (gate H-1). **The authoritative commit
  now exists: `37faade`.** Drop the mirrors rather than deduplicating them — rebasing onto `main`
  makes them empty, and a mirror kept alongside the real commit conflicts with it line for line.

### 8.2a The migration namespace was audited, 2026-09-15 — it is clean

Every migration file in every worktree was hashed and compared. **Nothing
collides.**

- **No two branches claim the same number for different files.**
- **No file with the same name differs in content across branches** — zero, over
  every migration in all twenty-five worktrees.
- Three branches carry `0120_exit_policy_evaluator_defect.sql` and two carry
  `0097`/`0098`; in both cases the downstream branch inherited the file from a
  landed track rather than writing its own, and the bytes are identical.
- Highest number anywhere: **0124** (`knowledge-time-provenance-repair`). `main`
  is at 0100.

Re-run it before integration; the sweep is a few minutes and the failure it
catches is the one that is hardest to read at merge time.

### 8.2b What `70cfc1f` touches that others also touch

- **`crates/dataplane/src/lib.rs`** — also tracks 3, 4, 9 and 20. Every one adds
  distinct `pub mod` lines (`census`, `backfill`, `fundamentals`, `qc`, …). The
  conflict is a list; take all the lines.
- **`crates/dataplane/Cargo.toml`** — also track 4. One dependency each.
- **`scripts/verify_plans.py`** — tracks 15, 19 and 24 each carry a copy, and all
  three are **byte-identical to `37faade`**: they mirrored it, none modified it.
  On rebase their copy becomes a no-op and `70cfc1f`'s two changes stand.

### 8.2c The merge hazard nobody listed: a silent deletion, 2026-09-15

Wave 1 nearly shipped one, and it is the reason §8.3's checklist is necessary
but not sufficient.

Track 1 replaces the hand-written `TAXONOMY` with `taxonomy()` generated from
`spec::catalog()`. Its branch predates `37faade`, so `catalog()` enumerates the
tool modules that existed when it was cut. Taking track 1's file — which §8.2
correctly tells you to do — took the catalogue from **72 tools to 60**, dropping
falsification, the three evidence tools, both web tools and the whole skill
library.

**Every test stayed green.** The taxonomy tests reconcile the catalogue against
the dispatch arms and the envelope lists; the merge removed the tools from all
of those at once, so nothing was left to disagree. `cargo test -p mcp-server-lib`
passed, `static_invariants` passed, `verify_plans.py` passed. A conflict stops
you; this does not.

**So: after any merge that makes a hand-written list generated, count the
generated output against the list it replaced.** For the catalogue that is one
example binary over `spec::catalog()`. The general form: a reconciliation test
proves two sides agree, never that either side is complete, and a merge is
exactly the event that can shrink both at once.

### 8.2d Three branches do not compile, and reported LANDED anyway

Found while merging, not before:

- **Track 16** (`execution-path-to-a-fill`): `crates/execution/src/lib.rs` had a
  literal `\u2014` in a Rust string (Rust wants `\u{2014}`), `orders.rs` had a
  bare `\n` on its own line, `SubmitResult` had no `Debug` while its own new
  hot-path tests call `expect_err`, and a test closure in
  `strategy-runtime/src/runtime.rs` captured a counter mutably and then read it
  five times. The first two look like file-writing artifacts; the last two are
  code that was written and never built.

These are cheap to fix and were fixed in the merge. The point is the process
one: **a track's own report that it landed is not evidence that its branch
builds.** Before the remaining waves, run `cargo build --workspace -j 2` on each
branch — or accept that the merge is where you will find out.

### 8.3 Verify after every merge, not at the end

A merge is done when all of these pass — failing any one means stop, not continue:

```bash
python scripts/verify_plans.py                       # counts, links, migration collisions
cargo test -p <every crate the merge touched>        # never --workspace
cargo xtask check-money-f64                          # must not gain a violation
cargo test -p invariants --test static_invariants    # must not lose a test
```

`check-money-f64` and `static_invariants` are **already red at HEAD** (§2.6). The question after
a merge is never "does it pass" but **"is it redder than before"** — record the counts before
merging so the comparison is possible.

### 8.4 Two things to settle before wave 1

- **Gate H-1.** Until the user commits the two `Cargo.toml` workspace-member lines and the
  untracked tool files, the catalogue stays at **60 tools instead of 73** and every track that
  needs them carries a mirror commit. Settling H-1 first removes work from every later wave.
- **`.gitattributes` and line endings.** Track 1 committed `.gitattributes` in its branch, so
  wave 1 introduces the LF pin repo-wide while existing migrations are CRLF on disk under
  `core.autocrlf=true`. Expect a renormalisation moment. **Do not renormalise while branches are
  outstanding** — decide it at wave 1 and let later waves inherit the result.

---

## 9. Verifying the board

Before you report, and after any edit to a plan file:

```bash
python scripts/verify_plans.py
```

It checks every plan set in one pass: per-phase task counts against the header, the
`**Acceptance:**`/`**Traces:**`/status-label counts against the task count, each MASTER's total
against its phase files and its own phase table, every relative link, task-id uniqueness, and —
the one that matters most with eighteen agents live — **migration-range collisions between sets
and against what is already on disk**.

**It was green when the board was created** (236 planned tasks across Sets P, S and T; highest
migration on disk 0060; no collisions). A green baseline is the point: if it fails after your
work, you broke something, and the output names the file. Do not add an exception to make it
pass — fix the file, or if the check itself is wrong, say so in your report.

---

## 10. Board log

| Date | Change |
|---|---|
| 2026-09-15 | **Integration started on branch `integration`, waves 0–3 landed.** The waves are being done off `main` so `main` stays green until the whole stack verifies. Two findings change the plan rather than just the code. First, the merge hazard in §8.2c: taking track 1's generated catalogue **silently deleted the thirteen tools H-1 landed**, 72 → 60, with every test still green, because the rows left both sides of every reconciliation at once. Restoring them as `ToolSpec` declarations also did **S-1.4**'s work for the twelve over-budget descriptions — 6.4 KB of prose moved to the authoring guide, and the seven tests guarding that prose moved with it. Second, §8.2d: **track 16's branch does not compile** — two file-writing artifacts and two genuine compile errors in code it reported as landed. Both were fixed in the merge. Also settled by measurement rather than by picking a side: the plan-hygiene audit and track 16 made contradictory claims about set-E 2.3 and 5.4; the code says track 16 is right (`risk_context.rs` computes `daily_loss_usd` and passes `RateSource::Tracked`; `hot_path.rs` stage 3 pushes `OrderIntent`s), so its claims were taken and the audit's objections are descriptions of the state it changed. `verify_plans.py` learns `PARTIAL`. |
| 2026-09-15 | **Set P Phase 2 landed on `main` (`70cfc1f`), and the migration namespace was audited clean.** The battery found a real defect on its first run against live data: `BTC-USD/coinbase/1m` contains **2026-08-13 twice** — 2,879 rows for 1,440 distinct minutes. Nothing had caught it because duplicates are not gaps, not bad prices and not stale, so every other check passes over them. Blast radius **A 0 · B 3 · C 1 · D 5** of nine series, so P-2.3's enforcement was deliberately **not built** — the phase sequences publication before enforcement, and that is now satisfied more strongly than asked. Also found and fixed: **`pg_strategy_revision` was not idempotent** — it used a constant hash, so it passed once against a database and failed forever after. Every live suite has now been run twice in a row; the rest are clean. |
| 2026-09-15 | **H-1 closed, `37faade`.** The baseline moved. Every track rebases onto `main`; anyone who mirrored the user's tree drops that commit rather than keeping it. §2.5a rewritten from a workaround into an instruction. The plan corpus was untracked too — tracks had neither the code nor the plans in their baseline. |
| 2026-09-15 | **Found an unclaimed block on the critical path.** Track 1 completed 5 of Phase 1's 20 tasks and its handoff named what came next, but nothing owned it: fourteen tasks, including the whole context-budget and reasoning-protocol halves. Track 26 dispatched for the first; S-1.14–S-1.20 recorded in §7 as the next dispatch. The lesson for the queue: **a landed track's partials are not automatically someone's work** — a phase is only finished when its file says so, and §7 now carries the residue explicitly rather than trusting that a successor picks it up. |
| 2026-09-15 | Tracks 10 and 12 resumed — both sit directly on unmet completion conditions. Track 10 closes **DONE-5** (the external MCP door is the last INV-16 exemption; the human and agent doors are already shut). Track 12 plus track 14 take the gate funnel from eleven judging on real numbers to sixteen, which is **Stage 2's exit**. Thirteen live. |
| 2026-09-15 | Track 1 fully verified: **all six suites cold-green in a private target dir** (27, 208, 30, 24, 116, 27), retracting its own earlier caveat so nothing rests on a shared-cache result. Its refined disk measurement — 4.5 GB with full graphs, 217.8 GB free — is now in §2.3 in place of the 2.4 GB figure. Track 25 (Phase 6, the desk) dispatched, completing the Set S fan-out. |
| 2026-09-15 | Queue pulled: tracks 23 (Phase 3, the catalogue) and 24 (Phase 5, the language) dispatched, track 2 (monitor) resumed. Phase 6 held — ten live is enough given the limit broke at sixteen. Track 1 followed up: five of six suites cold-verified in a private target dir, and it is **declining to call the sixth verified** until the cold run lands rather than relying on a `cargo clean` run that was demonstrably correct. That is the right standard. |
| 2026-09-15 | **Track 1 landed — the contracts exist**, so Set S Phases 2, 3, 4, 5 and 6 are unblocked; 21 (Phase 2) and 22 (Phase 4) dispatched, 3/5/6 queued on capacity. Its best find is a test that **asserted the defect**: `static_invariants.rs:632` checked `for_action("promote_model")`, a string no tool answers to, so it reported promotion as gated for exactly as long as the bug existed. It also measured the disk claim behind the shared-cache instruction and found it hollow — a private target directory is **2.4 GB**, not tens. §2.6 gains `cargo fmt --all -- --check`, which fails repo-wide at HEAD because `rustfmt.toml` sets nightly-only options that silently do not apply. And it committed `.gitattributes`, which had been untracked — so the LF pin the board claimed existed now actually does, in its branch. |
| 2026-09-15 | **Track 5 landed, and found that an instruction in every brief was corrupting builds.** §2.3 corrected: **never share `CARGO_TARGET_DIR` between worktrees** — colliding artifact hashes let worktrees overwrite each other's `.rmeta`, and a stale artifact can make a test pass that should fail. All six running tracks were told to switch to their own target directory and re-run anything they had already passed. Its own work: reconciliation wired with `Numeraire` as a closed enum with **no conversion API** (only `assert_comparable`, which refuses), `RealisedSeries` wrapping the ledger's own `ReturnSeries` so "same shape" is a compile-time fact, Gate 15 **leading** its verdict with count and window, and the divergence halt consulted **ahead of the idempotency cache** so a redelivered intent cannot ride a cached approval past it. Migration 0105 constrains `venue` to `'alpaca_paper'` so the internal paper engine can never become a Gate 15 venue — reconciling a simulator against itself measures nothing while producing rows that look like forward evidence. |
| 2026-09-15 | **Track 3 landed.** The census is published and the evidence clock is now a date, not a worry. Its two findings outrank the depth: every row's `knowledge_time` is false (track 20 dispatched) and `market_bar` is absent from the live database. **§2.1 corrected — the board's `.gitattributes` claim was false**, which is the worst kind of error a coordination document can carry: it told agents a live hazard was handled. §2.6's money-f64 line corrected to explain why two tracks measured different counts (`xtask` scans `git ls-files`). Track 4 resumed with all of it. Deribit blocked on ToS is the rule working, not a setback. |
| 2026-09-15 | **Track 15 landed.** Exits fire in the simulator. Its by-products were bigger: **every fill was charged zero commission** (gross reported as net, every run ever) and every `Trade` carried size zero — both fixed, both with the affected population separated at read time rather than rows rewritten, since `backtest_runs` is append-only by trigger. §2.5's failure list corrected (it omitted `check-money-f64`'s 16 violations and a failing static invariant). §2.5a added: **worktrees are cut from a baseline that lacks the uncommitted work they depend on** — H-1 biting in practice. §4 corrected: the oracle test does not guard the fill path, which I had implied it did. Track 19 dispatched for the live half of exits; track 14 resumed, since Gate 3 is the gate that would have caught the cost defect. |
| 2026-09-15 | **Rate-limit incident.** Sixteen concurrent Opus agents exhausted the session limit; all sixteen were killed mid-task. No work lost — worktrees intact, mostly uncommitted. Resumed five by priority (16, 3, 1, 15, 5) rather than all sixteen; eleven paused and resumable. §2.5 added with the capacity rule: **waves of about five, pulled by landings**, and commit per coherent piece rather than once at the end. |
| 2026-09-15 | **Track 6 landed.** `torch` resumes bit-identically unconditionally; `xgboost`/`lightgbm` only with sampling off, so they went into a separate `SAMPLING_SENSITIVE_FRAMEWORKS` whose `resume_support` **fails closed** — decisive because `hpo.py` samples subsample fractions below 1.0 on essentially every trial. `sklearn` and `garch` have no checkpoint boundary. AT-61 compares parameter bytes, prediction bytes and metric bytes, with no `allclose` anywhere, and was verified to fail in both directions. **Its most valuable output was correcting the handoff**: resume still restarts, because no adapter writes a checkpoint mid-run and nothing calls `plan_resume` — two blockers the handoff never listed. Dispatched as track 18. HANDOFF §1's stale "last ADR is P3-07" pointer fixed (P6-11 now). `ci.yml` and `ADR-INDEX.md` added to the shared-file list. |
| 2026-09-15 | Track 17: the two Set C items worth doing plus graceful shutdown and the JSON hot-path lint, which exists, enforces a real rule, and is in no CI job — which is why the WS defect survived. §8 added: integration is an open question and nobody merges to `main` without the user asking. Seventeen tracks. |
| 2026-09-15 | **Track 11 landed.** Closed roadmap gaps G-5, G-6 and human gate H-5 from evidence. Corrected four plan-set states in ROADMAP §2. Its most valuable output was not the audit: **there is no working path to a paper fill** — hot-path stage 3 is `Option::None`, armed automations have no caller, sizing returns `None`, exits are dropped on save, and nothing writes orders or fills to the database. That falsified ROADMAP §6's "costs a day" claim and moved Set E's execution items to Stage 0. Dispatched as **track 16**. Kill-switch restart bug sent to track 2; untracked catalogue files (13 tools, currently exempt from the `xtask` lints) sent to track 1; track 5 told T-0.4 is `BLOCKED:UPSTREAM` on track 16 and to build T-0.1…T-0.3 against an injected fill stream. |
| 2026-09-15 | Track 15: the simulator does not call `ExitTracker`, so exits declared at `definition_version` 1.3 are ignored in every backtest. Found while reconciling FEAT-005 for Stage 4; dispatched ahead of that work. Fifteen tracks live. |
| 2026-09-15 | Track 14: the gate stack's missing inputs (gates 3, 4, 10, 12). With track 5's Gate 15 that is the whole of the five dark gates, and Stage 2's exit criterion. §7 (the dispatch queue) added so no landing leaves the board idle. Fourteen tracks live. |
| 2026-09-15 | Third wave: tracks 12 (`regime_scopes` + the separation test SPEC §5.4 calls the difference between a real regime model and a decorative one) and 13 (the insight store's read path and decay). Both came from correcting ROADMAP §9 G-1, which wrongly recorded the regime engine as unspecified — it is specified and its rule tier runs; what was missing was configuration and validation. Thirteen tracks live. |
| 2026-09-15 | Second wave: tracks 9 (Set P Phase 3 alt data), 10 (AT-64 external MCP audit trail — the last INV-16 exemption), 11 (plan hygiene audit of Sets C, E, F, G). Eleven tracks live. |
| 2026-09-15 | Board created. Eight tracks dispatched: Set S Phases 0→1 and 7, Set P Phases 0 and 1, Set T Phase 0, and pack items §5.1, §5.2, §5.4. Migration reserve 0113–0120 opened for the pack tracks. |
