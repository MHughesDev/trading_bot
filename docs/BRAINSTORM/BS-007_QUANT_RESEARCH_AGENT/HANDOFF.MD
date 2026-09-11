# BS-007 agent handoff: quant research agent (2026-09-11)

This is for the next agent picking up the BS-007 work. Read this first, then
[00_INDEX.MD](00_INDEX.MD).

---

## 1. Where things stand

- **Branch:** `docs/bs-007-quant-research-agent`. It is **not pushed and has no PR**. It
  branches from `main` at `9f71f22`.
- **Commits on the branch** (all docs, no code):

| Commit | Content |
|---|---|
| `6d619c9` | Original BS-007 brainstorm (docs 00–12) |
| `d4dd76b` | Restructure into spec-ready subsystem docs (00–17 + research/R1–R4) |
| `6c9ae41` | ADRs 0024–0030 + Set L formal specs |
| `552c129`, `680b30c` | Specs README index (both case variants, see §6) |
| `40df804` | Remaining formal specs (DATA-006, FEAT-004/005/006, BACKTEST_SUITE_CORE_SPEC_V2) + link-case fix |

- **Phase 0 status:**
  - all 12 formal specs are written, with status **Proposed**;
  - all 7 ADRs are written, with status **Proposed**;
  - nothing is implemented.
- **Not done:**
  - the user hasn't accepted the ADRs;
  - the runtime spike hasn't run;
  - the backfill ops task hasn't started;
  - no plan sets have been written.

## 2. What BS-007 is (30-second version)

The platform is being made **agent-first**, around one long-running quant research
agent:
- **Runtime:** the Claude Agent SDK (Python) in a sandboxed container per research
  project. It reaches the platform only through `/api/*`, via a `tbot` SDK/CLI. Model
  calls go through a platform LLM proxy, so the container holds no secrets.
- **Enforcement:** research invariants are enforced by **platform services**, not
  prompts:
  - a Data API research cutoff (the holdout);
  - trial counting at job submission;
  - token scopes;
  - a `final_report` validator;
  - skill admission.
- **Supporting pieces:**
  - one feature engine;
  - two-layer strategies (Python research strategies + Strategy Language v2);
  - an honest backtest integration;
  - models via walk-forward prediction series;
  - an extended Set J research standard;
  - agent-created skills admitted by the platform;
  - free data sources only.

## 3. Locked decisions (by the user) — do not reopen without asking

D-01…D-13 in [00_INDEX.MD](00_INDEX.MD). Key ones:
- frontier models first;
- one agent, no modes;
- a container per project;
- **harness = Claude Agent SDK**;
- **two-layer strategies**;
- **free data only**;
- local embeddings;
- the agent creates skills, the platform admits them, core and global skills are used for
  every user, and every admitted skill is in the glossary;
- never trade model capability for tokens;
- research authority only: the agent never trades.

## 4. Map of the documents

| What | Where |
|---|---|
| Index, decisions, glossary, open questions | [00_INDEX.MD](00_INDEX.MD) |
| As-built baseline + gap register G-01…G-14 (audited 2026-09-10; re-verify line refs) | [01_GROUND_TRUTH.MD](01_GROUND_TRUTH.MD) |
| Normative subsystem docs with requirement IDs (RT-, CX-, JB-, DA-, FE-, ST-, BT-, MD-, EV-, KM-, SK-, TB-, UI-, AE-, B-) | 02–16 in this folder |
| Phases → plan sets L…R, requirement-ID scope per set, migration numbers, risks | [17_BUILD_ORDER.MD](17_BUILD_ORDER.MD) |
| Evidence and sources (informative) | [research/](research/) (R4 = token-efficiency audit) |
| Formal specs (the contracts plan sets implement) | `docs/specs/`: AGENT-001…004, COMP-005, COMP-006, DATA-005, DATA-006, FEAT-004…006, BACKTEST_SUITE_CORE_SPEC_V2 |
| ADRs | `docs/adr/0024`–`0030` (index updated in `docs/adr/README.md`) |

**Precedence:** the formal spec in `docs/specs/` is more concrete. The BS-007 subsystem
doc is the rationale. The research docs are evidence only.

## 5. Next steps, in order

1. **Ask the user to review and accept ADRs 0024–0030.** Flip `Status: Proposed →
   Accepted` in each ADR and in `docs/adr/README.md`.
2. **Ask the user to confirm the open questions** in 00_INDEX "Cross-cutting open
   questions", especially:
   - **Desk project design** (DATA-005 §4, ADR-0025): introduced during the cleanup and
     not yet confirmed by the user;
   - billing path for model calls (API key via proxy is assumed);
   - GPU availability;
   - monthly eval budget.
3. **Runtime spike** (17 §1; AGENT-001 §23). Verify every identifier marked *(verify)*
   in AGENT-001 against the pinned `claude-agent-sdk` version:
   - base-URL and auth-token env vars through the proxy;
   - session resume;
   - streaming input for steering;
   - PostCompact hook;
   - cache TTL options;
   - sub-agent caps;
   - the role header.

   Build it throwaway: container + minimal proxy + `tbot data bars` with the cutoff +
   `tbot jobs submit/wait` + a validated report + the pure-noise eval task. Update
   AGENT-001 with the findings.
4. **Ops: deep history backfill** (DATA-005 §11, S1). Target ≥ 3 years of 1m for BTC-USD
   and ETH-USD from free sources. Every later phase needs it.
5. **Verify the suspected `market_bars` data-loss bug** (DATA-005 §3). A task chip was
   offered to the user in the Claude app. **Don't run `OPTIMIZE` or drop anything on the
   production table without the user's approval.** The bug:
   - `clickhouse/02_bars.sql` is `ReplacingMergeTree(revision) ORDER BY (instrument_id,
     available_time)` with no `timeframe`, yet several timeframes are stored in the table
     (`crates/backtest/src/store.rs` filters by timeframe);
   - so 1m and 1h bars closing at the same instant may collapse on merge;
   - this may explain the thin 1h history (G-13).
6. **Reconcile Set K** (`docs/plans/plan-sets/set-K/MASTER.md`): part shipped with
   FEAT-003 Phase 1. Fold the remaining items into Sets L and M.
7. **Write plan set L** at `docs/plans/plan-sets/set-L/MASTER.md` + phase files. Follow
   the Set J format (`docs/plans/plan-sets/set-J/`). The scope is in 17 §2:
   - AGENT-001, AGENT-002 (core), AGENT-004 (skeleton);
   - COMP-005, COMP-006 (v1);
   - the DATA-005 core, and the AGENT-003 core skills and findings.

   Every task cites requirement IDs. Then add the set to `docs/plans/README.md`.
8. Later sets: M (engine truthfulness) and Q (research standard) in parallel, then N, O,
   R-a, P, R-b, plus the Track for live evaluation (G-11). The dependency graph is in
   17 §3.

## 6. Gotchas

- **Uncommitted user changes, do not touch or commit:** `Cargo.lock`, `Cargo.toml`,
  `apps/platform/src/main.rs`, `crates/config/Cargo.toml`, `crates/config/src/lib.rs`.
  They predate this work.
- **Case-insensitive filesystem vs git:**
  - git tracks `docs/specs/` in **lowercase**; Windows shows it as `docs/Specs`. Always
    write links as `docs/specs/…`, or they break on GitHub.
  - **`docs/specs/README.md` and `docs/specs/README.MD` are both tracked** and are one
    file on Windows. Keep them identical when committing:
    `h=$(git hash-object -w docs/specs/README.md); git update-index --cacheinfo 100644,$h,docs/specs/README.md; git update-index --cacheinfo 100644,$h,docs/specs/README.MD`.
- **Docker Desktop wasn't running** on 2026-09-11. See the phantom-socket crash-loop
  workaround in the project memory notes if it fails to start. The platform runs on port
  7080.
- **Cargo on this box:** build and test with `-j 2`; full parallelism exhausts the page
  file. Watch disk space (`target/` grew to 61 GB before). Never delete the Docker VHDX.
- **Numbers in the docs:** token and dollar figures (04, R4) are estimates, not
  measurements. Paper and arXiv results are as reported by their authors.
- **Free sources (ADR-0027):** confirm each provider's current terms, rate limits and
  US access before building an adapter. Binance returned HTTP 451 earlier.
- **Nothing in any spec gives the agent trading, automation, alias-promotion or holdout
  access.** Keep it that way (D-12, ADR-0025).

## 7. Conventions used

- Requirement IDs are stable. Cite them in plans and PRs (e.g. `RT-17`, `JB-02`).
- Enforcement tags: **(E)** platform service, **(H)** harness hook, **(A)** agent,
  checked by the validator or evals.
- Set J invariants INV-1/2/3 and ADR-0023 P1 hold everywhere.
- Commits end with `Co-Authored-By:` for the model used. The user asks before commits and
  pushes; branch off `main` for new work.
