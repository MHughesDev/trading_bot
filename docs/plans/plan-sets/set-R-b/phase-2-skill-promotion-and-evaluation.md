# Phase 2 — Skill promotion, curation and evaluation

**Completion: 67% (4 / 6 tasks), plus 2 `PARTIAL`.**

**Requirement IDs:** SK-11, SK-16, SK-17
**Spec:** [AGENT-003](../../../specs/AGENT-003-knowledge-memory-skills.md) §6.4
**Depends on:** Set R-a Phases 1 to 3.
**Migration range:** none.

A skill earns its way from a project to a user to everyone, and is retired when it stops earning. Every step has a human or an evidence condition, because a global skill is the one place a single mistake is replicated into every session.

---

### RB-2.1 Promotion to global meets all six conditions (SK-11) — L
`DONE` 2026-10-04 — `skills::lifecycle::check_promotion` over six *recorded* conditions (`skill_promotion_condition`, latest row per condition wins): a missing or failed one is refused by name, the reviewer may not be the author, and tainted provenance (computed from the memory store) refuses it whatever else is recorded; `POST /api/skills/{id}/conditions` and `POST /api/skills/{id}/promote` (`skills.admit`). Tests: `promotion_needs_all_six_and_names_the_missing_one`, `the_reviewer_may_not_be_the_author_and_taint_refuses_with_the_other_five`, and the pg flow. The producers of four conditions (cross-project evals, sanitiser/LLM check, security scan, owner consent) are external recorders; the platform's job, refusing an unrecorded one, is what is enforced.

Evals on tasks from other projects; a sanitiser (lint plus an LLM check via Batch); owner consent; a security scan; a reviewer who is not the author; no `tainted` provenance. All six are checked by the platform; the promotion is refused if any is unrecorded.

**Files:** `crates/skills` (additive), `crates/api` (outside this track)
**Acceptance:** a skill missing any one of the six is refused naming it; a tainted-provenance skill is refused even with the other five.
**Traces:** SK-11; AGENT-003 §6.4

### RB-2.2 project to user promotion is an approval — S
`DONE` 2026-10-04 — `POST /api/skills/{id}/promote` with `to: user` requires `skills.admit` (no agent holds it) and writes `skill_promotion_log` (who approved, from which scope) in the same transaction as the scope move (`move_scope()`); recorded in `skill_registry.skill_promotion_log` rather than `approval_requests` (ADR-RA-15). `promotion_to_global...` asserts the log row; `skills_admit_is_never_an_agent_scope` asserts an agent cannot reach the route.

`approval_requests.kind='skill_promotion'`, owner approval.

**Files:** `crates/api`
**Acceptance:** an agent cannot promote; the owner's approval is audited.
**Traces:** AGENT-003 §6.4

### RB-2.3 Quarantine — M
`DONE` 2026-10-04 — `POST /api/skills/{id}/quarantine` (`skills.admit`) calls `skill_registry.retire()`, which changes the status and writes the `skill_notice` in one statement; the glossary is a view so the skill leaves every glossary at once (`quarantine_leaves_every_glossary_in_one_statement_and_notifies_live_sessions`: gone for two different users, one notice for the session that had it resident, none for one that did not; a reason is required).

`POST /api/skills/{id}/quarantine` (admin) removes the skill from every glossary view at once and emits `skill_notice` (RA-2.6).

**Files:** `crates/api`
**Acceptance:** quarantining a global skill removes it from all glossaries in the same transaction and notifies live sessions (AGENT-003 acceptance 5).
**Traces:** SK-14; AGENT-003 §6.4

### RB-2.4 Skill A/B evaluation (SK-16) — L
`PARTIAL` 2026-10-04 — the design and the rule: `SequentialDesign` pre-registers `alpha`, `beta`, `p1`, `min_pairs` and `max_pairs` (reaching the cap is "no contribution"), `skill_eval_pair` stores paired and leave-one-out outcomes, and `SkillStore::sweep_retirements` archives a skill with enough exposures and a leave-one-out contribution at most zero or no activation (`retirement_and_the_model_guard_act_on_recorded_evidence`). **Missing:** the harness that runs the paired and leave-one-out arms at production model and effort (Set S evaluation harness, outside this track).

Paired, sequential, held-out, synthetic, at production model and effort; leave-one-out arms.

**Files:** `crates/harness/src/eval` (Set S territory)
**Acceptance:** a skill with no contribution is retired after the exposure threshold (AGENT-003 acceptance 6); the design pre-registers its stopping rule.
**Traces:** SK-16; AGENT-004

### RB-2.5 The library is re-evaluated before a model upgrade becomes the default (SK-17) — M
`PARTIAL` 2026-10-04 — `skills::lifecycle::check_model_change` and `skill_library_reeval` (0212): at session start a model with no completed library re-evaluation gets core skills only, skills failing under it are withheld and listed in the charter (ADR-RB-12); `retirement_and_the_model_guard_act_on_recorded_evidence`. **Missing:** the *change* of the default model is not itself refused (it is an environment variable with no event to hook; `agent/routes.rs` is outside this track), and nothing runs the library on the new model: `record_reevaluation` is fed by the missing harness.

A model default change is blocked until the library has been re-run on the new model.

**Files:** `crates/api`, `crates/harness`
**Acceptance:** changing the default model without a re-evaluation record is refused; skills failing under the new model are auto-disabled and listed.
**Traces:** SK-17

### RB-2.6 Retirement and deprecation — M
`DONE` 2026-10-04 — `retire_decision` (archive after N exposures with no activation or a leave-one-out contribution at most zero) is applied by `sweep_retirements` at every session start; a deprecated skill keeps `alias_of` (`skill_registry.set_alias()`, only for a deprecated skill) and resolves through it (`resolve_alias()`, at most eight hops, `GET /api/skills/item/{id}` returns `resolves_to`); an archived skill is not in the glossary view and so not resident (`retirement_and_the_model_guard...`, `a_deprecated_skill_resolves_through_alias_of_and_only_a_deprecated_one_can_be_an_alias`).

Archive after N exposures with no activation or a leave-one-out contribution at most zero; auto-disable on test failure after a platform, SDK or model bump; deprecated skills keep `alias_of`.

**Files:** `crates/api`, `crates/skills`
**Acceptance:** a deprecated skill resolves through `alias_of`; an archived one is not resident.
**Traces:** AGENT-003 §6.4
