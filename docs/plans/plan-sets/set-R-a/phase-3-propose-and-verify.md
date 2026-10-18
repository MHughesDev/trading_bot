# Phase 3 — Propose, verify and admit

**Completion: 38% (3 / 8 tasks), plus 5 `PARTIAL`.**

**Requirement IDs:** SK-04, SK-05, SK-06, SK-12, SK-13, SK-15
**Spec:** [AGENT-003](../../../specs/AGENT-003-knowledge-memory-skills.md) §6.3
**Depends on:** Phase 1.
**Migration range:** 0211 if needed.

The agent only *proposes*. A proposal cites a verified trajectory, is checked against the registry for near-duplicates (a similar enough one becomes a patch of the existing skill), and is verified by a
`skill_verify` job with an isolated Reviewer. The static scan is pure and is built here (RA-3.2); the rest needs the job service, an embedding model and the Set S evaluation harness.

---

### RA-3.1 Proposals and the near-duplicate check — M
`DONE` 2026-10-04 — `POST /api/skills/proposals` (`routes/skills::propose`, scope `research:artifacts`) calls `skills_verify::propose`: a proposal with no `source_ref` or a non-T1..T4 reason, or whose only sources are untrusted text, is refused with nothing written; a near-duplicate (lexical cosine at or above `SKILLS_PATCH_THRESHOLD`, default 0.85, configuration) or a proposal named like an existing skill becomes a patch of it with the bundle renamed to the target. `propose_verify_and_admit_with_every_refusal_naming_its_step`: a 0.97-similar proposal is a patch of `cost-check`, the same name is a patch, a core name is a patch of the core skill. The similarity is lexical, said so in the verdict (`mode: lexical`) until an embedder is wired (ADR-RA-11).

`POST /api/skills/proposals` runs a nearest-neighbour search over full bodies; similarity at or above `skills.patch_threshold` (default 0.85) makes it a patch of `target_skill_id`. A proposal with no cited trajectory is refused.

**Files:** `crates/api` (outside this track), `crates/skills/src/registry.rs`
**Acceptance:** a proposal 0.9 similar to an existing skill becomes a patch; a speculative proposal without a trajectory is refused (AGENT-003 acceptance 3); the threshold is configuration, not a constant.
**Traces:** SK-04; AGENT-003 §6.3

### RA-3.2 The static scan and lint — M
`DONE` 2026-10-04 — `skills::lint::lint_bundle` is now called by `Draft::check`, which `skills_verify::propose` and `admit_proposal` run (the `lint` and `procedure` steps), and by `sync_core` (hard rules only for core, ADR-RA-07). `propose_verify_and_admit_with_every_refusal_naming_its_step` proposes a body with `BTC-USD` and `0.7312` over the real route code and gets `rejected` with both findings and their lines. The lint stays lexical: it cannot see an equity ticker, which is why the Reviewer step exists.

No ticker symbols, dates or fitted-looking constants; no network, credentials or filesystem escape; no `allowed-tools`, hooks or `!` shell; description at most 100 tokens; body at most 500 lines.
Pure, over a bundle's files.

**Files:** `crates/skills/src/lint.rs` (new), `crates/skills/src/lib.rs` (one `pub mod` line)
**Acceptance:** a skill containing `BTC-USD`, a date literal or a four-decimal constant fails with the line; a bundle whose front matter enables tools, hooks or shell is refused (reusing `core`'s refusal); a clean skill passes; each rule has a mutation-checked case.
**Traces:** SK-06, SK-08, SK-12; AGENT-003 §6.3 step 1, acceptance 4

### RA-3.3 The `skill_verify` job — L
`PARTIAL` 2026-10-04 — the verify step, built: `skill_verdict` rows per step, `POST .../verify` (computes the replay verdict from recorded pairs, the analysis-code tests and the savings condition, and records a missing Reviewer as a *failed* step), `POST .../steps` for results the platform cannot compute, `POST .../admit` refusing with the first unmet step named, `registry::reviewer_context` (the Reviewer sees only the bundle, the tests and the task descriptions; no parameter a trajectory could arrive through). **Missing:** a `skill_verify` job kind with a per-skill token and compute budget (`crates/jobs`), and the isolated Reviewer sub-agent actually being launched; until then the Reviewer's verdict is recorded by a person through `.../steps`.

A job kind with a per-skill token and compute budget that runs the lint, the unit tests, the replay, the trigger evals and the coexistence check, and writes a verdict the admission reads. The Reviewer sees only the bundle, the tests and the task descriptions.

**Files:** `crates/jobs`, `crates/api`, `crates/harness` (outside this track)
**Acceptance:** a skill failing any step is not admitted; the verdict names the failing step; the Reviewer's context contains no trajectory content (a test on the assembled prompt).
**Traces:** SK-05; AGENT-003 §6.3

### RA-3.4 Replay verification: with and without the skill — L
`PARTIAL` 2026-10-04 — the design and its rule: `skills::lifecycle::replay_verdict` (paired, sequential sign test, pre-registered `SequentialDesign` with `alpha`, `beta`, `p1`, `min_pairs`, `max_pairs`; at least three variants; a skill never activated fails; one that changes nothing fails; reaching the cap is "no contribution"), `skill_eval_pair` (0212) and `verify` computing the `replay` step from the stored pairs. **Missing:** the harness that *runs* the source task and variants with and without the skill at production model and effort (Set S evaluation harness territory); today the pairs arrive through `POST .../pairs`.

The source task plus at least two variants on synthetic instruments, paired and sequential, at production model and effort. The skill must be activated and must change the outcome.

**Files:** `crates/harness/src/eval` (Set S territory)
**Acceptance:** a skill the model never activates fails; a skill that changes nothing fails; the design is paired with a stated stopping rule.
**Traces:** SK-05; AGENT-004

### RA-3.5 Analysis-code skills: truncation, noise null, leakage trap — M
`DONE` 2026-10-04 — `skills::quality` (truncation check with the first differing timestamp; noise null at 5% nominal with a 7% ceiling; seeded noise) runs the skill through the existing sandboxed runner, and `skills_verify::analysis_tests` calls it for `kind: analysis-code` bundles (the `tests` step, required before admission via `registry::required_steps`). Live runs against Python: `a_bfill_skill_fails_truncation_naming_the_first_differing_timestamp`, `a_causal_skill_passes_truncation`, `a_calibrated_test_passes_the_noise_null_and_one_that_finds_structure_in_noise_fails` (3 live tests pass, ~26 s). Not reused: Set M's `research_strategy.truncation_test`, which re-runs a *strategy's position series*; this is the series-transform form (ADR in the report).

For `kind=analysis-code`: the PIT truncation test (reuse Set M's `research_strategy.truncation_test`, M-1.4), a noise-null test (false-positive rate at most 7% at 5% nominal) and a leakage-trap task.

**Files:** `crates/skills`, `apps/skill-runner`
**Acceptance:** a `bfill` skill fails truncation naming the first differing timestamp (AGENT-003 acceptance 4); a skill that finds structure in white noise more than 7% of the time fails.
**Traces:** SK-06; AGENT-003 §6.3 step 6

### RA-3.6 The savings condition (T5) — S
`PARTIAL` 2026-10-04 — `skills::lifecycle::savings_condition` (at least 20% fewer tokens or tool calls at equal success; a cheaper-but-worse skill fails; mutation-checked by the 10%-fails case) and `verify` records a `savings` step, required through `registry::required_steps`, for a bundle that declares `claims: savings`. **Missing:** the paired runs that supply the costs (RA-3.4's harness).

Tokens or tool calls reduced at least 20% at equal success, measured by RA-3.4's paired runs.

**Files:** `crates/harness`
**Acceptance:** a skill that saves 10% fails; at equal success only.
**Traces:** SK-05

### RA-3.7 Proposal triggers T1 to T4 and the single Stop-hook nudge — M
`PARTIAL` 2026-10-04 — `skills::lifecycle::qualifies` (T1 repeated three times in a project or twice across, T2 at least 15 tool calls or an error-then-fix **and** platform-verified, T3, T4, and the refusals that beat every trigger: speculative, one-off, single-tool wrapper, symbols or tuned values, untrusted-only source), `StopNudge` (at most once per session; a second Stop does not nudge), and the proposal path refuses an untrusted-only proposal (`registry::check_proposal`). **Missing:** the Stop hook itself: `local_driver` does not build a `TriggerEvidence` from a finished run or deliver the nudge text, because the run's tool-call and verification counters are not exposed to a hook yet.

T1 repetition (the same procedure three times in a project or twice across projects), T2 hard-won and verified (at least 15 tool calls or an error-then-fix, plus a platform-verified outcome), T3 pitfall, T4 human request;
never speculative, one-off, single-tool wrappers, content with symbols or tuned values, or proposals sourced only from untrusted text. The Stop hook nudges at most once per session (SK-15).

**Files:** `crates/api/src/agent`, `crates/harness` (outside this track)
**Acceptance:** a session nudges once and a second Stop does not; a proposal whose only source was untrusted text is refused.
**Traces:** SK-15; AGENT-003 §6.3

### RA-3.8 A skill bill of materials per run — M
`PARTIAL` 2026-10-04 — `SkillStore::bill_of_materials` and `GET /api/skills/runs/{session}/bill` list a run's skills by name and version hash with exposed/activated, read from `skill_run` alone (`session_start_mounts_core_records_exposure_once...`), and the only counters are `skill_run` rows. **Missing:** the run's own receipt (the `agent_runs` record) does not embed the list, so the receipt and the bill are two reads.

Record which skills (name, version) a run had exposed and activated; usage statistics live only in the registry.

**Files:** `crates/api`, `migrations/0211_skill_usage.sql` (new if needed)
**Acceptance:** a run's receipt lists its skills by version hash; no counter exists outside `skill_run`.
**Traces:** SK-13
