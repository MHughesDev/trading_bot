# Phase 1 — The registry and admission

**Completion: 100% (5 / 5 tasks).**

**Requirement IDs:** SK-02, SK-03, SK-08, SK-09
**Spec:** [AGENT-003](../../../specs/AGENT-003-knowledge-memory-skills.md) §3, §6.1; [ADR-0028](../../../adr/0028-skill-registry-and-platform-side-admission.md)
**Depends on:** nothing inside this set.
**Migration range:** 0210.

Today a skill is a directory of files (`skills/core/<category>/<name>/SKILL.md`) read by `skills::core`, plus a separate Python-transformation library (migration 0060). There is no registry:
no versions, no scopes, no glossary *view*, no run record, and nothing that can admit a skill. This phase builds the tables and the one transaction through which a skill becomes visible, because
ADR-0028's central claim is that admission is platform-side and writes the glossary row in the same transaction.

---

### RA-1.1 The registry schema — L
`DONE` 2026-10-04 — `migrations/0210_skill_registry.sql` (schema `skill_registry`) plus `0211_skill_registry_store.sql` (bundles, verdicts, notices, standing-change functions, grants) and `0212_skill_evaluation.sql`. Applied on a scratch Postgres 17 (5433) behind the same three dependencies the platform has for them (the `app_role` role, 0044; `agent_memory` for 0225): `pg_registry.rs` (9 tests, the constraints and the shadowing trigger) and `crates/api/tests/skills_registry_flow.rs` (`app_role_can_do_what_the_platform_does_and_nothing_that_would_bypass_admission`: `app_role` reads the glossary view, inserts `skill_run`, executes `admit()`/`retire()`, and every direct write to `skill`, `skill_version` or `skill_bundle` is a permission error). Not run: the platform's *whole* migration chain on this machine (0055 and 0122 need pgvector, which the 5433 server lacks); the registry migrations depend on nothing but `app_role`. Verified 2026-10-05: `skills_registry_flow` 13 of 13 pass and `pg_registry` 9 of 9 pass against Postgres 17 on 5433.

Migration 0210: `skill` (id, name, scope `core|global|user|project`, owner and project ids, category, status `provisional|admitted|quarantined|deprecated|archived`, `alias_of`),
`skill_version` (immutable, content hash, description, body hash, token count), `skill_run` (state `exposed|activated|...`, helpfulness vote), `skill_proposal`, `project_skill_selection`, and
`skill_glossary` as a **view** over admitted and provisional rows. A unique index on `name` across core and global, so a lower scope cannot shadow them. Every table has `GRANT ... TO app_role`.

**Files:** `migrations/0210_skill_registry.sql` (new)
**Acceptance:** applied to a scratch database with its dependencies; `app_role` can read the view and write `skill_run`, and cannot UPDATE or DELETE `skill_version` (`SET LOCAL ROLE app_role` test); a user-scope skill named like a core skill is refused by the index.
**Traces:** SK-02, SK-09; AGENT-003 §3; ADR-0028

### RA-1.2 Admission is one transaction that writes the glossary row — M
`DONE` 2026-10-04 — `skill_registry.admit()` is now the only way in as a **grant**: 0211 makes `admit`, `retire`, `set_admitted`, `move_scope` and `set_alias` `SECURITY DEFINER` (pinned `search_path`) and revokes `INSERT`/`UPDATE` on `skill` and `INSERT` on `skill_version` from `app_role`, so "no INSERT into skill outside admit" is not a convention (ADR-RA-12; mutation: re-granting `INSERT` makes the `app_role` test fail on the `INSERT INTO skill` line). The one caller is `api::skills_store::SkillStore::admit` (function and bundle in one transaction), reached by `sync_core` (every core skill), `skills_verify::admit_proposal` and the tests. The glossary is a view, so a skill is visible in the statement that admits it (`an_admitted_skill_is_in_the_glossary_in_the_transaction_that_wrote_it`, and a rollback leaves nothing visible).

`admit(proposal)` inserts the `skill` and its `skill_version` and, in the same transaction, the glossary-visible status; there is no code path that makes a skill visible without it. Because the glossary is a
view, 'in the same transaction' is satisfied by construction and the test proves it rather than asserting it.

**Files:** `crates/skills/src/registry.rs` (new), a store in `crates/api` (outside this track)
**Acceptance:** an admitted skill appears in the glossary view in the transaction that wrote it; a rolled-back admission leaves no visible skill; there is no `INSERT` into `skill` outside `admit`.
**Traces:** SK-02; AGENT-003 §6.3

### RA-1.3 `skills.admit` is never an agent scope — S
`DONE` 2026-10-04 — `crates/invariants/tests/skills_admit_is_never_an_agent_scope.rs` (6 tests): `skills.admit` is in `NEVER_FOR_AGENTS`, not in `RESEARCH_SCOPES`, refused by `validate_agent_scopes` alone and among legitimate scopes, named by migration 0038's trigger; every `pub async fn` in `routes/skills.rs` is either on a closed list of eight read/research-scope handlers or begins with `require_admit`; every mounted `/api/skills/*` route names a classified handler; no MCP tool is named for an admin action and the registry tool file mentions no admin path. Mutation-checked: disabling the taint check in `check_promotion` fails `the_reviewer_may_not_be_the_author_and_taint_refuses_with_the_other_five`; the handler-guard scan reads source so deleting a `require_admit` fails it by name (that deletion was not mutation-run this session).

SK-03: the agent proposes; admission is a platform-side write. A token-scope test enumerates every scope an agent token can carry and fails if `skills.admit` is among them, or if any route that admits is reachable by one.

**Files:** `crates/api/src/auth/scopes.rs` (outside this track), `crates/invariants/tests` (new file)
**Acceptance:** the scope table has no agent-grantable `skills.admit`; mutation check: granting it makes the test fail.
**Traces:** SK-03; ADR-0028

### RA-1.4 Description tokens and body lines enforced at admission — S
`DONE` 2026-10-04 — one definition: `skills::core::description_tokens_over` and `body_lines_over` are the only places the 100-token and 500-line limits are compared; `core::validate` (core loading, and therefore admission), `lint::lint_bundle` and `registry::Draft::check` all call them (`registry::tests::the_limits_have_one_definition` counts the comparisons in the source). `a_101_token_description_and_a_501_line_body_are_refused_with_the_count` and the exact-limit pass. The schema repeats the same bounds as CHECKs (0210).

`skills::core` already enforces at most 100 estimated tokens and 500 lines for core skills. Move the checks to a shared function and call it from admission so a user skill cannot exceed what a core skill may.

**Files:** `crates/skills/src/registry.rs`, `crates/skills/src/core.rs` (additive)
**Acceptance:** a 101-token description and a 501-line body are refused at admission with the count; core loading and admission share one function (one definition test).
**Traces:** SK-08; AGENT-003 §6.3

### RA-1.5 Core skills load into the registry at deploy — M
`DONE` 2026-10-04 — `SkillStore::sync_core` loads `skills/core/**` as `scope=core, status=admitted, created_by=release`, idempotent on content hash; its caller is `agent::skills_mount::session_start`, the first thing every agent run does (ADR-RA-14), so it cannot be the unreached boot hook it would otherwise risk being. `core_sync_is_idempotent_and_a_changed_bundle_is_a_new_version_that_leaves_the_old_readable`: loading twice is a no-op, a changed `SKILL.md` is a second version with the first still readable, the glossary lists every core skill, and a release's older bytes never overwrite an admitted delta above them (KM-05).

`skills/core/**` are loaded with `scope=core, status=admitted`, signed by the release. Idempotent on content hash: a changed bundle is a new `skill_version`, never an edit.

**Files:** `crates/skills/src/core.rs` (additive), `apps/platform` boot (outside this track)
**Acceptance:** loading twice is a no-op; a changed `SKILL.md` produces a second version and leaves the first readable; the glossary lists every core skill.
**Traces:** SK-01; AGENT-003 §6.1
