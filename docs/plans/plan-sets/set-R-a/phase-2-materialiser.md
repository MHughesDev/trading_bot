# Phase 2 — The materialiser

**Completion: 50% (3 / 6 tasks), plus 2 `PARTIAL`, 1 `BLOCKED:HUMAN`.**

**Requirement IDs:** SK-01, SK-07, SK-09, SK-13, SK-14
**Spec:** [AGENT-003](../../../specs/AGENT-003-knowledge-memory-skills.md) §6.2
**Depends on:** Phase 1.
**Migration range:** none.

`skills::core` is the v0 materialiser: it mounts every core skill and a generated glossary and says it has no scopes. The v1 materialiser (AGENT-003 §6.2) resolves the visible set per
`(user, project, project_version)`, selects a resident set of at most 40 in a deterministic order, writes it to a read-only mount, records which skills were exposed and verifies each bundle's hash.
The resolution and selection are pure and are built in `crates/skills/src/materialiser.rs` (2026-10-04); the registry-backed inputs and the session hook are the production half.

---

### RA-2.1 Scope resolution — M
`DONE` 2026-10-04 — `materialiser::visible` is fed by `SkillStore::visible` (rows from `skill_glossary` for this user and project) in `session_start`; the scope, quarantine and shadowing tests are the earlier unit tests, plus `propose_verify_and_admit_with_every_refusal_naming_its_step` (a project skill is visible to its project and not to a session with none).

From the visible rows: all `core`, all `global`, the user's `user` skills, the project's `project` skills. A lower scope cannot shadow a core or global name; a collision is an error naming both, never a silent winner.

**Files:** `crates/skills/src/materialiser.rs` (new), `crates/skills/src/lib.rs` (one `pub mod` line)
**Acceptance:** a project skill named like a core skill is refused; a quarantined skill is invisible; a user's skill is invisible to another user; resolution is a pure function of its input.
**Traces:** SK-01; AGENT-003 §6.2 step 1

### RA-2.2 The resident set: at most 40, deterministic, frozen per project version — M
`PARTIAL` 2026-10-04 — `select` is called by `session_start` over the registry rows and the frozen selection read from `project_skill_selection`; `SkillStore::freeze_with` writes a new project version when a project skill is admitted. **Missing:** a conversation has no project (ADR-RA-16), so a session passes none and its frozen selection is empty: core and pinned global are resident, and project skills reach a session only through the glossary and search. The 40-skill cap, determinism and reorder-invariance are the unit tests (cap mutation-checked earlier).

Core first, then pinned global, then the project's frozen selection, up to 40 in total; order by tier then name; descriptions verbatim from the admitted version. Over the cap, the lowest tier is cut and the cut is reported. Same input, same bytes.

**Files:** `crates/skills/src/materialiser.rs`
**Acceptance:** 41 candidate skills yield 40 resident and a report naming the cut one; two calls with one input produce identical bytes; reordering the input does not change the output; core skills are never the ones cut unless core alone exceeds 40 (which is an error).
**Traces:** SK-01, SK-07; AGENT-003 §6.2 step 2

### RA-2.3 The mount and the on-demand glossary — S
`DONE` 2026-10-04 — `skills::mount::write_mount` writes the resident set (any scope) and `skill-finder/GLOSSARY.md` for the visible set under `<workspace>/.claude/skills`, read-only on disk, verifying every bundle's digest first and removing a skill directory the resident set no longer holds. Its caller is `session_start`. Tests: `two_sessions_get_byte_identical_mounts_that_are_read_only_and_list_every_skill`, `a_quarantined_skill_is_removed_at_the_next_mount_and_leaves_the_glossary`, missing finder / missing bundle refusals. `core::materialise` remains as the v0 fallback when the registry is unavailable.

Write the resident skills to the read-only mount and `GLOSSARY.md` inside `skill-finder/`. `skills::core` already generates the glossary from a skill list; generalise it to the visible set.

**Files:** `crates/skills/src/materialiser.rs`, `crates/skills/src/core.rs`
**Acceptance:** the glossary lists every visible skill (resident or not) with scope and status; resident bytes are identical for two users with the same visible set.
**Traces:** SK-09; AGENT-003 §6.2 step 3

### RA-2.4 Exposure rows — S
`DONE` 2026-10-04 — `SkillStore::record_exposures` writes one `exposed` row per resident skill at session start and none for a skill that is only in the glossary; a read of a mounted `SKILL.md` through the agent's own `read_file` writes the `activated` row once (`workspace_tools::activated_skill`, recorded in `local_driver::execute_tool`). `session_start_mounts_core_records_exposure_once_and_a_read_activates` (counts equal the resident set, a repeat start adds none, activation once, `write_file` into the mount refused).

Record a `skill_run` row with `state='exposed'` for each resident skill; the provisional-to-admitted rule (RA-4.5) reads them.

**Files:** `crates/skills`, `crates/api` store (outside this track)
**Acceptance:** materialising a session writes one exposed row per resident skill and none for a non-resident one.
**Traces:** SK-13; AGENT-003 §6.2 step 5

### RA-2.5 Verify each bundle's hash and signature — M
`BLOCKED:HUMAN` 2026-10-04 — everything that does not need the key is built: `skills::signing` (Ed25519 over the canonical core manifest; `sign`, `verify`, `check_core`), `cargo run -p skills --bin sign_core`, and `session_start` refuses an unsigned or tampered core set **once `TBOT_SKILLS_SIGNING_PUBKEY` is set** (with none, nothing is required and nothing is claimed). `skills::signing` tests (3) pass: a signed manifest verifies, a changed or extended one does not, a key with no signature file refuses. **Human step:** (1) choose where the release secret lives (a password manager or an offline file; the platform never reads it); (2) generate 32 random bytes as 64 hex characters, e.g. `python -c "import secrets; print(secrets.token_hex(32))"`; (3) run `TBOT_SKILLS_SIGNING_SECRET=<hex> cargo run -p skills --bin sign_core -- skills/core`, which writes `skills/core/MANIFEST.sig` and prints the public key; (4) commit `MANIFEST.sig` with the release and set `TBOT_SKILLS_SIGNING_PUBKEY=<printed key>` in the platform's environment; (5) re-sign whenever a core skill changes (the signature covers every skill's digest, so a stale one is refused).

Hash verification exists in `skills::core`. The **signature** needs a release signing key. **Human step:** the owner chooses where the key lives (the platform has no signing key today) and provides the public half to the deploy.

**Files:** `crates/skills/src/materialiser.rs`
**Acceptance:** a bundle whose bytes differ from its recorded hash aborts materialisation before anything is written (exists); an unsigned core bundle is refused once a key is configured.
**Traces:** SK-01; AGENT-003 §6.2 step 6

### RA-2.6 Mid-session revocation as a notice — M
`PARTIAL` 2026-10-04 — `SkillStore::retire` writes the status change and a `skill_notice` row in one statement; `agent::skills_mount::pending_notices` turns the notices for skills a session has resident into system messages, and `local_driver::prepare_native` appends them to the thread as `[harness]` turns, never touching the system prompt (`quarantine_leaves_every_glossary_in_one_statement_and_notifies_live_sessions`: gone for every user at once, one notice for a session that had it resident, none for one that did not). **Missing:** the constrained-decoding transport (`!native`) rebuilds its prompt from the loop and has no thread to append to, so a session on it learns at its next start.

The orchestrator emits a `skill_notice` event appended as a system message; the system prompt is never edited; the skill is removed at the next session start (SK-14).

**Files:** `crates/api/src/agent` (another track is editing it)
**Acceptance:** quarantining a resident skill produces one notice in every live session that has it exposed; the system prompt bytes are unchanged.
**Traces:** SK-14; AGENT-003 §6.2
