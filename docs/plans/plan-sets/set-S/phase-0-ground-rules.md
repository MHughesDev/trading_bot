# Phase 0 — Ground rules

**Completion: 0% (0 / 6 tasks)**

**Requirement IDs:** GR-01…GR-06.
**Spec:** none directly — this phase makes the other eight executable.
**Depends on:** nothing.
**Blocks:** every other phase.
**Migration range:** none.

This is not agent-platform work. It is the six things that are false today and would make
every later task lie: a dirty tree nobody can branch from cleanly, an approval defect that
leaves model promotion ungated, thirteen tools that exist only on one machine, and eleven
specs whose acceptance sections cannot be closed by the procedure that closes acceptance
sections.

---

## Tasks

### S-0.1 Sort the working tree with the user — S
`NOT STARTED`

~20 files are modified on `main` and several are untracked. `BS-007/HANDOFF.MD` §6 names
files an agent must not commit: `Cargo.lock`, `Cargo.toml`, `apps/platform/src/main.rs`,
`crates/config/Cargo.toml`, `crates/config/src/lib.rs`. Ask the user which of the rest is
theirs, then land the platform's own work in one commit and leave the user's alone. Do
not `git add -A`.

**Files:** working tree only.
**Acceptance:** `git status --short` shows only files the user has confirmed are theirs;
`main` builds and `just test-unit` is green at the same commit the tracks branch from.
**Traces:** MASTER §5 (agent contract), BS-007 HANDOFF §6.

### S-0.2 `promote_model_version` is not approval-gated — fix and prove — S
`NOT STARTED`

`crates/ledger/src/audit.rs:98` lists `promote_model` in `PROMOTION_ACTIONS`; the tool is
named `promote_model_version` (`crates/mcp-server/src/taxonomy.rs:149`), so
`Envelope::for_action` returns `None` and a promotion — an action AGENT-001 §15 says needs
a human — is ungated on both drivers today. Fix the name, then add the test that makes the
class of defect impossible: every name in every envelope list is a catalogue tool, and
every tool declaring an envelope appears in its list.

**Files:** `crates/ledger/src/audit.rs`, `crates/mcp-server/src/taxonomy.rs`, a new test in
`crates/mcp-server`.
**Acceptance:** `Envelope::for_action("promote_model_version") == Envelope::Promotion`; the
new test fails when a list entry names a tool that does not exist, and when a tool's
declared envelope is missing from its list. Adversarial: a test asserts the *old* spelling
no longer matches anything.
**Traces:** AGENT-006 §0 defect 1, §2.6, acceptance §10.2.

### S-0.3 Commit the untracked catalogue, under the tracked path casing — S
`NOT STARTED`

`crates/mcp-server/src/tools/falsification.rs` and `research_tools.rs` hold 13 tools —
the newest and most load-bearing part of the catalogue — and are untracked. The nine Set S
specs and this plan set are untracked. Git tracks `docs/specs/` **lowercase** while Windows
shows `docs/Specs`; committing the new specs under the capital-S path yields two
directories on a case-sensitive checkout and breaks every link in this plan.

**Files:** `crates/mcp-server/src/tools/{falsification,research_tools}.rs`,
`crates/api/src/routes/{evidence,web_and_skills}.rs`, `crates/backtest/src/forensics.rs`,
`docs/specs/AGENT-005…014`, `docs/specs/COMP-007`, `docs/adr/003{4,5,6,7}-*.md`,
`docs/plans/plan-sets/set-S/**`, `migrations/0059`, `0060`.
**Acceptance:** `git ls-files docs/specs/AGENT-0*.md` lists all fourteen AGENT specs under one
lowercase directory; `cargo xtask check-money-f64` and `check-bars-v1-frozen` pass *after*
`git add` (they scan `git ls-files` and silently skip unstaged files); `just test-unit`
green.
**Traces:** AGENT-006 §0 ("untracked in git"), MASTER §5 hazards.

### S-0.4 Give the eleven specs the acceptance form the evidence chain needs — M
`NOT STARTED`

`docs/procedures/execute-plan.md` step 3c closes the chain requirement → criterion →
evidence by writing a test name into a spec's `§6.1.x … — Verified by: [—]` slot. The Set S
specs use a plain `## N. Acceptance` numbered list, which has no slot. Convert each to the
repo form without changing a single criterion's meaning: `6.1.A [ ] <condition> (verifies
§N.M) — Verified by: [—]`.

**Files:** `docs/specs/AGENT-005…014`, `docs/specs/COMP-007` (acceptance sections only).
**Acceptance:** every criterion in every Set S spec (AGENT-005 Parts I and II, AGENT-006…014, COMP-007 — the counts are in MASTER §7) carries an addressable id and an empty `Verified by:` slot;
`cargo test -p api --test auditor_suite` unaffected; a spot check of three criteria shows
wording unchanged. No criterion is added, removed or softened in the conversion.
**Traces:** `docs/procedures/execute-plan.md` §3c, `add-spec.md` §6.

### S-0.5 Update every index that now points at nothing — S
`NOT STARTED`

**Files:** `docs/specs/README.md` (target table `Plan set` column → `S` for the eleven specs;
mark AGENT-001…004 and COMP-006 as superseded per each spec's header),
`docs/plans/README.md` (index row for Set S), `docs/adr/README.md` (rows for 0034, 0035, 0036, 0037 — added 2026-09-14, verify),
`docs/ml-platform-context-pack/decisions/ADR-INDEX.md` (the five P7 rows are already
appended — verify), `docs/FOLDER_MAP.md` (it still shows `adr/` ending at 0013).
**Acceptance:** `cargo test -p api --test auditor_suite` green; every link in Set S MASTER
resolves from a case-sensitive checkout; no index row points at a missing file.
**Traces:** `add-plan.md` step 6, `add-adr.md`.

### S-0.6 Correct the governance register, and reserve the migration ranges — S
`NOT STARTED`

`docs/governance/COMPLACENCY_LOG.md` lists as CRITICAL that
`crates/api/src/auth/session.rs` accepts any non-empty bearer token. **That is stale** —
the file now resolves the token against `sessions` with an expiry check and returns the
session's scopes, and `BearerToken::permits` gates on them. Correct the entry rather than
leaving a false CRITICAL that hides real ones. Then record the Set S migration ranges
(MASTER §5) in the log's header so concurrent tracks cannot collide, and add the two
genuine dead-code entries this set will remove (`Section::Tools`, `needs_model_summary`).

**Files:** `docs/governance/COMPLACENCY_LOG.md`.
**Acceptance:** the auth entry is corrected with the evidence (`session.rs:57-88`) and dated;
the migration ranges are recorded; the two dead-code entries name the Set S task that
removes them.
**Traces:** COMP-007 §8.6 depends on scope enforcement being real; AGENT-005 §4.5.

---

## Exit criteria

- `main` is clean apart from files the user has claimed, and every track can branch from it.
- Model promotion is approval-gated, with a test that prevents the whole class of
  name-drift defect.
- The catalogue is fully tracked under one path casing.
- Every acceptance criterion in the eleven specs is addressable and has an empty `Verified by:` slot.
- Every index resolves; no false CRITICAL remains in the governance register.
