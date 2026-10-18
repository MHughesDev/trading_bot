# Skills lifecycle — Set R-a

**Completion: 27% left (14 / 24 tasks). 14 done, 9 partial, 1 BLOCKED:HUMAN, 0 not started.**

**Status:** PLANNED (2026-10-04, track F-RESERVED) — this file was the reserved letter in [ROADMAP §5](../../ROADMAP.md#5-set-succession-what-the-reserved-letters-still-mean) until today.
**Stage:** [5](../../ROADMAP.md#4-the-stages) — accumulated knowledge; Phase 1 can start now
**Derived from:** [AGENT-003](../../../specs/AGENT-003-knowledge-memory-skills.md) §3, §6, §8, [BS-007 13_SKILLS](../../../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/13_SKILLS.MD), [ADR-0028](../../../adr/0028-skill-registry-and-platform-side-admission.md), ROADMAP §5, OPEN-TASKS Part C3
**Requirement IDs:** SK-01…SK-10, SK-12…SK-15, UI-06.
**Migration range:** 0210–0211.
**Scope class:** End-state architecture.

---

## 1. Why this set exists

The agent gets better at a task only if what it learned survives the session, and the only form of that which this platform can honestly support is a skill: a reusable procedure, admitted by the
platform after it has been shown to work, with a record of how often it was exposed and used. The registry is the hard part. Without it, `skills/core/` is a folder of files that every session mounts in full, a
lower-scope skill cannot exist, nothing can be admitted, and nothing says which skill was in front of the model when a result was produced.

## 2. Scope

| Area | Today | This set delivers |
|---|---|---|
| Registry | files under `skills/core/` and a Python-library table (0060) | versioned, scoped skills with a glossary view, run records and proposals |
| Admission | none | one platform-side transaction; `skills.admit` is never an agent scope |
| Materialiser | v0: mount all core + glossary | scoped, at most 40, deterministic, frozen per project version, hash-verified |
| Propose and verify | none | a proposal path, a static lint, a verify job with an isolated Reviewer and replay |
| Search and UI | none | hybrid search over bodies, the REST surface and the Skills page |

### Out of scope

- Promotion to global, the curator, A/B evaluation and library re-evaluation (Set R-b Phase 2).
- The playbook as a registry skill (KM-05, Set R-b RB-1.4).
- Python-transformation skills (`create_skill`, `run_skill`; Set S ADR-P7).

### Excluded because something else already owns it

| What | Owner | Evidence |
|---|---|---|
| Core skill sources and the v0 materialiser | Set L / track LEGACY | `crates/skills/src/core.rs`, `skills/core/**` |
| Playbook admission and episodic and semantic memory | Set S Phase 4 | S-4.x |
| The Python skill library | Set S ADR-P7 | migration 0060, `crates/skills` |

## 3. Decisions made while planning (ADR rows)

Each row records the alternative that was rejected. They are this set's own ADR index; promote one to `docs/adr/` when a task relies on it.

| ID | Decision | Rejected alternative | Why |
|---|---|---|---|
| ADR-RA-01 | The glossary is a SQL **view** over the registry; there is no glossary table. | A materialised glossary table refreshed on admission. | A table can be stale; a view cannot, so SK-02 ('no admitted skill absent from the glossary') holds by construction. |
| ADR-RA-02 | The materialiser's resolution and selection are pure functions in `crates/skills`; the database read and the mount write are adapters. | A single async materialiser that queries as it selects. | Byte-identical output (SK-01) is only testable if selection has no I/O. |
| ADR-RA-03 | Over the 40-skill cap the lowest tier is cut and the cut is reported; core never silently. | Truncate by name order. | An alphabetical cut would drop a core skill because its name sorts late. |
| ADR-RA-04 | Provisional to admitted needs 20 exposures with a non-negative balance. | Leave N open. | AGENT-003 open question 3; a stated default is configuration and can be changed, an open one blocks the task. |
| ADR-RA-05 | Admission and the registry tables are `app_role`-granted in their own migration, tested with `SET LOCAL ROLE app_role`. | Rely on owner-connected tests. | Owner tests cannot see a missing grant (see the schema-grants note). |
| ADR-RA-06 | The lint treats any four-or-more-decimal literal, date literal or ticker-shaped token as a finding, and a skill can only suppress one with a reason the Reviewer reads. | A regex list the author can extend. | SK-06's point is that tuned values and instrument names do not belong in a reusable skill; a suppression with a reason is auditable. |
| ADR-RA-07 | Release-signed core skills are held to the lint's *hard* rules (capability, description tokens, body lines) and not to its content rules (ticker, date, tuned constant). | Hold core to every rule and rewrite `skill-authoring`. | `skill-authoring` has to name `BTC-USD` and `0.73` to forbid them; the content rules exist for agent-authored skills whose text nobody signed. A test asserts that no *other* core skill names a forbidden thing. |
| ADR-RA-08 | The session mount is `<conversation workspace>/.claude/skills`, written read-only by `skills::mount::write_mount`; the local driver may *read* it through a path check in `workspace_tools::skills_path` and can never write it, because `write_file` still goes through `Workspace::resolve`, which refuses a path in no area. | Mounting under `inbox/skills/` (a user-visible area), or teaching `Workspace::resolve` a sixth area. | `inbox/` is the user's copy of their own inputs; a sixth area changes a harness type that finished tracks depend on. A named read-only exception in the IO layer is the smaller change and keeps the write refusal structural. |
| ADR-RA-09 | An agent's proposal is always **project scope**. User and global scope are reached only by promotion (`skills::lifecycle::check_promotion`, an approval logged in `skill_promotion_log`). | Letting a proposal name its own scope. | A proposal that could ask for `global` would make the six promotion conditions optional. |
| ADR-RA-10 | A version's hash **is** its bundle digest (`core::bundle_digest`), and a bundle's files are stored as JSONB text in `skill_bundle` (0211), append-only. | A separate version counter, or files on disk. | Content addressing makes "the same bytes are the same version" and "a changed SKILL.md is a new version, never an edit" true by construction; files on disk are a second store that can drift from the hash. |
| ADR-RA-11 | The near-duplicate check and `skills search` are **lexical** (cosine over word counts; Postgres full-text) and say so in the response, until an embedder is wired to the registry (RB-1.6 decision: pgvector, ADR-RB-06). | Blocking RA-3.1 and RA-4.1 on the embedder. | A stated lexical fallback is honest and testable now; a dense score replaces it behind `registry::similarity`'s signature. |
| ADR-RA-12 | `admit()`, `retire()`, `set_admitted()` and `move_scope()` are `SECURITY DEFINER`, and `app_role` loses `INSERT`/`UPDATE` on `skill` and `INSERT` on `skill_version` (0211). | Relying on the shadowing trigger and on callers using the function. | RA-1.2's "no INSERT outside admit" must be a grant, not a habit; `pg app_role_can_do_what_the_platform_does_and_nothing_that_would_bypass_admission` fails if any direct write works. |
| ADR-RA-13 | An admitted proposal is **provisional** and becomes admitted after N exposures with a non-negative balance (ADR-RA-04). The settle and the archive sweeps run at every session start. | Admitting straight to `admitted`; a separate scheduler job. | New procedures earn their place by use (AGENT-003 §6.3); running the sweeps where sessions start means no scheduler has to be remembered, and the result is the same. |
| ADR-RA-14 | Core skills are synced into the registry at the first session start after a deploy (`SkillStore::sync_core`), not from `apps/platform/src/main.rs`. A release's older bytes never overwrite an admitted delta above them. | Editing the platform's boot code (outside this set's territory). | The sync is idempotent and cheap, and its caller is the one path every session takes, so it cannot be unreached the way a boot hook could be. |
| ADR-RA-15 | A project-promotion approval is recorded in `skill_registry.skill_promotion_log`, not `approval_requests`. | Reusing `approval_requests.kind = 'skill_promotion'`. | That table is project-bound with a foreign key to `research_projects` and is answered by the desk's approval inbox, which does not know what a skill is. |
| ADR-RA-16 | A conversation has no project (`agent_conversations` carries none), so a session's viewer passes no project: it sees core, global and its own user skills, and project skills are reached through the API. | Inventing a conversation-to-project link. | When conversations gain a project, `skills_mount::session_start` is the one place to pass it. |

## 4. Phase summary

| Phase | Title | Tasks | Migrations | State |
|---|---|---|---|---|
| 1 | [The registry and admission](phase-1-registry.md) | 5 | 0210 | 5 / 5 done |
| 2 | [The materialiser](phase-2-materialiser.md) | 6 | none | 3 / 6 done |
| 3 | [Propose, verify and admit](phase-3-propose-and-verify.md) | 8 | 0211 if needed | 3 / 8 done |
| 4 | [Search, the Skills page and SKILL.md procedures](phase-4-search-ui-procedures.md) | 5 | none | 3 / 5 done |

### Territory note

> **Verified 2026-10-05:** skills lib 114, research lib 150, harness lib 708, api lib targeted 40, mcp-server-lib catalogue tests (incl. the `propose_skill` descriptor budget), static_invariants 35, `skills_admit_is_never_an_agent_scope` 6, pipeline-builder guard 2, `skills_registry_flow` 13 and `pg_registry` 9 on Postgres 5433, `quality_live` 3. Anything needing pgvector remains compile-checked only.

Built by F-RESERVED on 2026-10-04 (pure parts of `crates/skills`): the scope resolution and resident-set selection (RA-2.1, RA-2.2) and the static lint (RA-3.2). The registry tables (RA-1.1) were drafted as
migration 0210 and validated against a scratch database.

**Built by track R-SKILLS on 2026-10-04 (the reach):** the registry and the materialiser now have callers. `agent::skills_mount::session_start` runs at the start of every agent run (`local_driver`, fresh
and resumed): it loads `skills/core`, verifies the release signature when a key is configured, syncs the core set into the registry, settles provisional skills and archives the unused, reads what the user may see,
applies the model guard, selects the resident set, writes the read-only mount to `<workspace>/.claude/skills`, records one exposure per resident skill and hands the charter the resident descriptions. A read of a
mounted `SKILL.md` is the activation; a quarantine reaches a live session as a notice. Propose, verify and admit are `routes/skills.rs` (`skills.admit`-guarded) over `skills_store.rs` and `skills_verify.rs`, with
`skills::{registry, lifecycle, mount, quality, signing}` holding every rule as a pure function. New files: `crates/skills/src/{registry,lifecycle,mount,quality,signing}.rs`, `crates/skills/src/bin/sign_core.rs`,
`crates/api/src/{skills_store,skills_verify}.rs`, `crates/api/src/agent/skills_mount.rs`, `crates/api/src/routes/skills.rs`, `crates/mcp-server/src/tools/skills_registry.rs`, migrations 0211 and 0212, the
`tbot skills` commands, and the procedure template.

**R-WIRE, 2026-10-05:** the Skills page (RA-4.3) is built over the served routes (`frontend/src/pages/SkillsPage.tsx`) and needs only the `App.tsx` route and nav entry; `glossary` and `search` now return `skill_id` (ADR-RW-12, in set R-b's index).

What is still not true: the **Reviewer** and the **paired replay runs** are not launched by anything (RA-3.3, RA-3.4, RA-3.6: a person or the Set S harness records their results through `.../steps` and `.../pairs`);
the **Stop-hook nudge** has no evidence feed (RA-3.7); the **signature** waits for a key (RA-2.5); and a session passes no project because a conversation has none (ADR-RA-16).
