# AGENT-007: Research Memory — what the agent knows, how it learns, and how knowledge enters context

> **Status:** **Partially implemented** — Set S phase 4. Re-checked 2026-09-21.
>
> The seven entry kinds, their provenance columns and the brief are built, with the refusals asserted against a live Postgres. The retrieval and decay halves are open.
>
> **The live count is in [`docs/plans/plan-sets/set-S/phase-4-memory-and-evaluation.md`](../plans/plan-sets/set-S/phase-4-memory-and-evaluation.md), and the programme total is in [`docs/plans/PLAN.md`](../plans/PLAN.md).** It is deliberately not restated here: a number written into a spec is a number nobody recomputes, which is how this line came to say "ready to build" about code that was already serving traffic. `crates/invariants/tests/the_plan_counts_what_the_plans_say.rs` holds those two files honest; nothing holds a hand-copied number here.
**Date:** 2026-09-14
**Author:** Claude (Fable 5.1)
**Revises:** AGENT-003 (knowledge, memory, skills) — §2 layers, §5 findings, §6 skills, §7 profiles are replaced by this document; §3 storage and §4 embeddings are amended
**Depends on:** AGENT-005 (the `Reference` section and its budget), AGENT-006 (`ToolResult.ids`, `memory.*` and `report.*` tools)
**Related:** ADR-0028 (skill admission), ADR-0033 (agent as a conversation), `migrations/0055_knowledge_plane.sql`, `migrations/0041_agent_conversations.sql`

---

## 0. Why this spec exists

An agent that starts every session from a bare goal is not a researcher; it is a very fast intern with amnesia. Today:

- The only cross-turn memory is `conversations::prior_context` (`crates/api/src/agent/conversations.rs:339`), a per-turn "you concluded: …" string built from the model's own final prose. It is ignored by the hosted driver (AGENT-005 §0) and, on the local driver, prepended to the goal with no budget and no structure.
- `record_finding` (`crates/harness/src/drive.rs:1728`) collects findings inside one run and throws them away at the end. Nothing on the next turn, the next conversation, or the next user knows they existed.
- Negative results — the trials that failed, and *why* — are in the trial ledger as rows, not as knowledge anyone can ask for. `find_similar_trials` (`crates/mcp-server/src/tools/falsification.rs`) matches a config hash; it cannot say "this idea died of capacity in June".
- The workspace has a `memory/` area (`crates/harness/src/workspace.rs:32` `Area`) that nothing writes to with any structure.
- AGENT-003 specifies findings, profiles, analogs and a skill materialiser, all "Proposed, not implemented". `migrations/0055_knowledge_plane.sql` exists and is the storage base to extend, not replace.
- "Skills" means two unrelated things: Python `run(payload)` transformations (`crates/skills`, built) and SKILL.md procedures (ADR-0028, not built). This spec names them apart.

The research brief (`docs/research/quant-agent-toolbox-uncommon-tools.md` Theme 7) found no published system that keeps a negative-result ledger running inside a live research org — and that agents that only remember successes silently re-derive the field's 60% non-replication rate. Memory is the difference between an agent that is *capable* and one that is *understanding*: the second knows what has already been established, what has been refuted, and how sure it should be.

## 1. Principles

- **P1 — Memory is written by tools, not by the model.** A memory entry is created from a `ToolResult` (AGENT-006 §2.4) that carries ids, `n` and `se`. The model may *propose* a finding; the harness verifies the ids exist and the numbers came from a result before it is stored. Free-text memory is how hallucinations become institutional.
- **P2 — Every entry has provenance, validity and confidence.** Who/what produced it, from which data window, valid until when, with what evidence class. A finding without these is a rumour.
- **P3 — Negative results are first-class.** "Tried and failed, because X" is stored with the same care as a success, with a typed failure class, because it is the entry most likely to save the next session's budget.
- **P4 — Retrieval is budgeted, ranked, and explained.** What enters context is chosen against the `Reference` share (AGENT-005 §4.5), ordered by relevance × recency × confidence, and each retrieved item says *why* it was retrieved. A small model gets fewer, shorter items; it never gets none.
- **P5 — Conflicts are surfaced, never merged.** Two findings that disagree are both retrieved, marked as conflicting, and the disagreement is itself a research lead.
- **P6 — Procedures are admitted, not accreted.** A playbook the agent writes goes through propose → verify → admit (ADR-0028). Nothing the agent writes reaches its own resident context without a check, because self-written skills measurably make agents worse (SkillsBench, cited in ADR-0028).
- **P7 — Memory decays and is re-verified.** A finding about a market is a claim about a period. It carries `valid_until`, and a scheduled job re-runs the tool that produced it; a stale finding is flagged when cited.

## 2. The three memories

| Memory | What it holds | Written by | Read into context as | Lifetime |
|---|---|---|---|---|
| **Episodic** | the effect/input log of every run (AGENT-010 §5), and a deterministic **turn summary** per turn | the harness, automatically | the opening message (AGENT-005 §3.2), and on demand via `memory.recall_turn` | the conversation, then archived |
| **Semantic** | findings, negative results, instrument profiles, signal registry, platform-defect notes, user preferences | `record_finding` (verified), `memory.record_negative_result`, scheduled profilers, `report_evaluator_defect`, `ask_user` answers | the **task brief** (§4) in the `Reference` section, and on demand via `memory.search` | until `valid_until`, then re-verified or marked stale |
| **Procedural** | playbooks (`PLAYBOOK.md` procedures), Python skills (`crates/skills`), and the workflows compiled from proven playbooks (AGENT-006 §6) | proposed by agent or human, admitted by the platform | a **resident listing** (name + one line, ≤ 40 entries) in the charter's tail; bodies fetched by `get_playbook` | until deprecated by vote or by eval regression |

### 2.1 Episodic: the turn summary contract

[DO] Replace `prior_context`'s prose with a **structured turn summary** the harness writes at the end of every run from the findings ledger and the effect log — not from the model's final message. Fields (AGENT-001 §10.2's list, made a schema):

```json
{ "turn": 3, "goal": "…", "outcome": "finished|refused|escalated|fenced|cancelled",
  "findings": [{"id": "f_…", "claim": "…", "ids": {"experiment_id": "…"}, "n": 812, "se": 0.04, "confidence": "established|suggestive|refuted"}],
  "negative_results": [{"ticket_id": "…", "class": "weak_effect", "why": "…"}],
  "trials_spent": 14, "n_eff_after": 37, "gate_states": {"exp_…": "gate_2"},
  "open_questions": ["…"], "rejected_ideas": [{"idea": "…", "reason": "…"}],
  "budget": {"tokens": 41200, "usd": 0.83}, "yield_ref": "y_…", "stage": "report", "current_step": "…" }
```

Rendered for the next turn's opening message at ≤ 600 tokens (the harness truncates lists by confidence, never the ids). Stored in `agent_turn_summaries` (new table, keyed by conversation and turn). `conversations::prior_context` becomes a renderer over this table.

### 2.2 Semantic: entry kinds and their schemas

All kinds share: `id`, `kind`, `subject` (instrument / hypothesis family / strategy / platform component), `claim` (≤ 300 chars), `evidence_ids` (map), `n`, `se`, `confidence`, `evidence_class` (`direct_stat | derived | model_summary | human`), `data_window {start, end}`, `valid_until`, `provenance {run_id, tool, tool_version, template_hash}`, `created_by {agent_model | user_id}`, `superseded_by?`, `conflicts_with[]`, `stale: bool`, `embedding`.

| Kind | Extra fields | Written by |
|---|---|---|
| `finding` | `hypothesis_family_id` | `record_finding` after verification (§3.1) |
| `negative_result` | `failure_class`, `ticket_id`, `retry_condition?` (e.g. "execution adapter for Kraken exists") | `memory.record_negative_result` (AGENT-006 §5.8), derived from the funnel stage that failed |
| `instrument_profile` | `coverage {timeframes, start, end, gaps}`, `regime_now`, `liquidity {spread_bps, adv}`, `tail {hill_alpha}`, `seasonality_survivors[]`, `analogs[]` | a scheduled profiler job running the `analysis` templates (AGENT-006 §5.2) weekly per initialised instrument |
| `signal_registry` | `signal_spec`, `original_source`, `original_effect`, `replicated_effect`, `decay_curve_handle` | backfilled from the open-source factor libraries; extended by `register_result` |
| `platform_defect` | `component`, `claims_to_guarantee`, `reproduction` | `report_evaluator_defect` |
| `user_preference` | `scope` (global/instrument/strategy), `statement` | `ask_user` answers and explicit "remember that…" turns, always attributed to the user |
| `open_question` | `why_it_matters`, `blocking: bool`, `value: low\|med\|high`, `opened_by: run_id`, `status: open \| closed{by, how: answered\|superseded\|withdrawn}`, `related_finding_ids[]` | `lifecycle.open_question` (AGENT-013 §9) and the harness from `WorkingState.open_questions` at run end; closed by the platform when a later finding cites it |

#### 2.2.1 [DO] Who computes `conflicts_with`

P5 says conflicts are surfaced and never merged, and §2.2 gives every entry a
`conflicts_with[]` — but nothing in the spec filled it, which would have made P5 a field
that is always empty. On every `finding` write, the platform runs a **contradiction check**
against `established` findings sharing the same `subject` and an overlapping horizon:

1. Candidate set: same subject, same instrument class, overlapping `data_window` or an
   explicitly comparable one, `stale = false`.
2. A candidate contradicts the new finding when the claim's **direction** opposes it on the
   same measured quantity — the direction comes from the structured `evidence_ids` and the
   cited numeric field, not from parsing the prose. Two findings that disagree on a number
   whose confidence intervals overlap are **not** a contradiction; they are one finding with
   a wider interval.
3. On a real contradiction both rows are linked, both keep their confidence, and **neither is
   downgraded automatically.** A later finding is not more true for being later.
4. The pair is retrieved together into every brief matching either (§4) and offered in the
   desk as a resolution lead (AGENT-012 §7).
5. Resolution is a *research act*, not an edit: an agent or a human runs the comparison that
   distinguishes them and records a third finding citing both. Only that supersedes.

This is the memory-poisoning guard. Without it one wrong `established` finding becomes an
authoritative premise for every later run, and the honesty stack sits on an unchallenged
claim.

[DO] Extend `migrations/0055_knowledge_plane.sql` with these columns where absent; add `agent_turn_summaries` and a `memory_retrievals` log (what was retrieved for which run, and whether it was cited — the input to AGENT-009's memory-recall metric).

### 2.3 Procedural: three things, three names

- **Skill** stays what `crates/skills` built: a Python `run(payload) -> dict` transformation, no network, admitted by static scan today (ADR-0028's verify job is the [DO] in AGENT-006 §3.3 / this spec §5).
- **Playbook** (new): a `PLAYBOOK.md` — when to use, preconditions, ordered steps naming catalogue tools, decision points, what "done" looks like, known failure modes. ≤ 500 lines; description ≤ 100 tokens. Stored in `playbooks/<category>/<name>/` with `playbook.yaml` (tier, applicability filters: asset class, timeframe range, hypothesis families), versioned, voted, deprecatable — the existing `skills.propose/patch/vote/deprecate` tools are renamed to `playbooks.*` for this kind.
- **Workflow tool** (AGENT-006 §6): a playbook that has been run ≥ N times with a clean verdict is *compiled* into a server-side workflow by an engineer. The playbook stays as documentation of the workflow; the model calls the tool. This is how procedural knowledge stops costing tokens.

### 2.4 Retention: memory that only grows stops being memory

Every entry kind here accumulates, and the profiler (§3.3) writes one row per instrument per
week forever. Retrieval ranks, so a store that grows without bound does not fail loudly — it
degrades quietly, which is worse: the brief slowly fills with old, narrowly-true findings and
the agent's understanding gets *worse* over time while every dashboard stays green.

| Kind | Live | Then | Never deleted |
|---|---|---|---|
| `finding` | until `valid_until` | re-verified (§P7) or marked `stale`; stale-and-uncited for 180 d → `archived` | anything cited by a sealed trial ticket or a dossier |
| `negative_result` | indefinitely | — | all; this is the cheapest entry in the store and the most likely to save a budget |
| `instrument_profile` | latest only in retrieval | superseded rows kept 90 d, then archived | the latest per instrument |
| `user_preference` | until the user deletes it | — | — |
| `platform_defect` | until fixed | archived with the fix's commit | — |
| `open_question` | until closed | closed-and-uncited for 180 d → `archived` | any question a continuous session's agenda item cites (AGENT-014 §4.3) |

`archived` means out of retrieval and out of the brief, still addressable by id, still
resolvable in the desk so an old dossier's citations do not break. **Nothing an evidence
chain points at is ever hard-deleted** — that is the same append-only discipline the ledger
has, applied to the knowledge that cites it.

A monthly job reports store size, archive rate and the share of retrievals that were cited
(§4), so a degrading brief is visible as a number before it is visible as bad research.

### 2.5 [DO] The same finding, recorded twice

Two conversations investigating the same thing will record the same conclusion, and a brief
that shows it twice spends its budget saying one thing. On write, after the contradiction
check (§2.2.1), a **near-duplicate check**: same subject, same measured quantity, overlapping
data window, and claims whose confidence intervals overlap. A near-duplicate does not create
a row — it **increments a corroboration count** on the existing entry and appends the new
run's ids to its evidence.

That is strictly better than deduplication: the second independent measurement is *evidence*,
and a finding corroborated by three runs on different windows should outrank a singleton in
retrieval, which is exactly what the count now lets the ranker do.

## 3. Writing memory

### 3.1 [DO] `record_finding` becomes verified

`drive.rs:1728` currently accepts any `(finding, evidence)` strings. New behaviour:
1. Parse `evidence` for ids; every id must be in the run's `seen_ids` (the same set `finish_task` checks, `drive.rs:1830`). Unknown id → rejection with the correction "evidence must name ids returned by a tool in this run".
2. Any number in `claim` must appear in a result carrying that id, within rounding; else `evidence_class` is downgraded to `derived` and the finding is stored with `confidence: suggestive`, never `established`.
3. Store to semantic memory with provenance from the run and `valid_until = data_window.end + policy(kind)` (default 90 days for market findings; platform defects never expire until fixed).
4. Return the `finding_id`; the findings block (AGENT-005 §4.4) lists ids so later claims can cite them.

### 3.2 [DO] Negative results are derived, not declared

When the funnel refuses a gate, a study fails its objective, or a workflow verdict is `reject`, the platform writes a `negative_result` with `failure_class` from a closed enum and `retry_condition` from the verdict's detail (e.g. `capacity_insufficient` → "retry if AUM target ≤ collapse_aum"). AGENT-013 §8.2 extends the enum with `leakage`, `protocol_unstable`, `fragile_parameters` and `regime_specific`, each derived from a named ticket. The agent's `memory.record_negative_result` tool can only *annotate* an existing entry (`notes`), never create one with a class the funnel did not produce. This is P1 applied to failure: the model cannot mislabel a dead idea as "just needs more capacity".

### 3.3 [DO] Instrument profiles by schedule

A `profile_instruments` job (the durable job service, `crates/jobs`) runs weekly for every initialised asset: `describe_returns`, `estimate_volatility`, `detect_regime`, `estimate_liquidity_cost`, `seasonality`, `find_analogs` (AGENT-006 §5.2) at 1h and 1d, and writes one `instrument_profile` entry per instrument, superseding the last. Cost is bounded (templates, no trials).

### 3.4 [DO] The user's own knowledge

Answers to `ask_user`, and turns that begin "remember that" / "from now on", are stored as `user_preference` with `scope`. They are retrieved into every brief for that scope at top rank. The user can list and delete them (`GET/DELETE /api/memory/preferences`). This is the cheapest large gain in "understanding": the agent stops re-asking and stops re-deciding what the user already decided.

## 4. Reading memory: the task brief

[DO] At run start, before the first model call, the harness assembles a **brief** and pushes it as `Reference` blocks (AGENT-005 §2.1 L4 / §4.5 share 0.20):

| Slot | Query | Budget (share of Reference) |
|---|---|---|
| user preferences in scope | `scope ∈ {global, instrument, strategy}` | up to 15% |
| instrument profile(s) | instruments named in the goal or constraints | up to 25%, most recent only |
| relevant findings | hybrid search over goal + constraints; filter `stale = false`; rank by similarity × recency × confidence; `established` before `suggestive` | up to 35% |
| negative results | same query, `kind = negative_result`, plus any whose `retry_condition` is now satisfied (flagged "retry condition met") | up to 15%, always ≥ 1 if any match |
| open questions in scope | `kind = open_question`, `status = open`, subject match; `blocking` first (AGENT-013 §9) | up to 10% |
| conflicts | any retrieved pair with `conflicts_with` | always included, both sides, marked |
| resident playbook listing | applicability filters vs. goal | ≤ 40 lines, in the charter tail rather than Reference |

Each item is rendered as one line plus ids: `[f_31a, established, n=812, 2026-04→2026-06] BTC-USD 1h: EMA-cross edge is spanned by momentum (residual t=0.4). why: matches "EMA" and instrument`. The brief for a 7B is the same query with a smaller share and one-line items only; for the frontier tier, `detail` fields are included.

`memory.search(query, kinds[], k)` and `memory.recall_turn(turn)` exist for on-demand reads with the same renderer and are subject to `max_tool_result_bytes` like any tool.

[DO] Log every retrieval to `memory_retrievals {run_id, entry_id, rank, why, cited: bool}`; `cited` is set when a later `record_finding`/`finish_task` names the entry's id. This is the memory-recall signal AGENT-009 scores.

## 5. Admission for procedures (ADR-0028, made concrete)

[DO] `playbooks.propose` creates a candidate in `playbooks/_proposed/`. A `playbook_verify` job runs: (1) static checks (every named tool exists in the catalogue, every step is tool-shaped, ≤ 500 lines, description ≤ 100 tokens); (2) an A/B replay on the eval suite's relevant tasks (AGENT-009 §3) with and without the playbook resident, on the tier it targets; (3) admission only on non-inferiority *and* at least one task improved; (4) a human approves (`playbooks.admit` is never on an agent token). Rejections are stored as `negative_result` of kind `playbook` with the eval diff, so the same playbook is not re-proposed.

Python skills get the same job with (2) replaced by unit tests the proposer must supply and the existing scan.

## 6. Embeddings

[DO] A local embedding model served by the model-inference sidecar (`apps/model-inference`), chosen by an eval over the finding corpus (AGENT-003 §11's open question is answered by measurement, not by picking). Until it exists, and whenever the sidecar is down, retrieval falls back to the lexical scorer already in `registry::search` (`crates/harness/src/registry.rs:527`) over `claim + subject`. Retrieval must never block a run on an embedding service.

## 7. Build order

| Phase | Items |
|---|---|
| 1 | turn summaries (§2.1) replacing `prior_context`; verified `record_finding` (§3.1); `memory_retrievals` log |
| 2 | semantic schema on 0055 (§2.2); derived negative results (§3.2); the brief (§4) with lexical retrieval |
| 3 | user preferences (§3.4); instrument profiler job (§3.3) once AGENT-006 phase 2 templates exist |
| 4 | playbooks + admission job (§5); rename `skills.propose/patch/vote/deprecate` → `playbooks.*` |
| 5 | local embeddings (§6); signal registry backfill |

## 8. Acceptance

> **Form note (Set S task S-0.4, 2026-10-03).** Converted from a plain numbered
> list to the repository's acceptance form so `docs/procedures/execute-plan.md`
> step 3c can close the chain requirement -> criterion -> evidence. No criterion
> was added, removed, reworded or softened; each kept its position and its number
> (`1a` is `AC-1a`), so item *n* of the old list is `AC-n`. An unticked box with
> `Verified by: [--]` means nothing has been proved, which is the honest default.
> The form is the one AGENT-005, AGENT-006 and AGENT-008 use.

- [x] AC-1: A finding whose `evidence` names an id no tool returned is refused; one whose number is not in the cited result is stored `suggestive`, never `established`. - Verified by: `harness::memory::verify::tests::{a_finding_citing_an_id_no_tool_returned_is_refused,a_number_absent_from_the_cited_result_is_stored_suggestive,a_number_present_in_the_cited_result_is_established,a_number_from_an_uncited_result_does_not_back_the_claim,a_claim_with_no_ids_is_accepted_but_can_never_be_established}` (the verdict is computed and stored, S-4.2), `harness/tests/conformance.rs::{a_finding_may_not_cite_an_identifier_the_session_never_saw,finish_task_evidence_may_not_cite_an_identifier_the_session_never_saw}` at the loop, and `harness::eval::cases::a_fake_id_in_finish_task_is_refused` as a replay case with a broken twin (`a_fake_id_accepted_is_the_broken_version`). *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-2: A second conversation about the same instrument receives, in its brief, the first conversation's established findings and negative results, with ids, under the Reference budget, on a 7B profile and on the frontier profile. - Verified by: `harness::memory::brief::tests::a_second_conversation_receives_the_first_findings_with_ids_on_both_tiers` (7B and frontier budgets), `harness::memory::turn::tests::a_second_turn_receives_the_first_conclusions_with_ids_on_both_tiers`, and against Postgres `api/tests/turn_summary_round_trip.rs::a_second_turn_receives_the_firsts_conclusions_with_ids` and `memory_reaches_the_run::an_answer_given_in_turn_one_is_in_turn_twos_brief_at_the_top`. **Partial:** two real conversations on the same instrument through a live model have never run (gate H-2); what is proved is the assembly and the store, not a model reading the brief. Written, not re-run in this pass (needs live Postgres or a full crate build); CI runs it. *(Filled 2026-10-04, S-8.9.)*
- [x] AC-3: A negative result cannot be created with a `failure_class` the funnel did not produce. - Verified by: `harness::memory::negative::tests::{an_agent_cannot_create_a_negative_result_with_a_class_the_funnel_did_not_produce,every_class_is_a_platform_event_and_the_enum_is_closed,the_class_and_the_retry_condition_both_come_from_the_source}` and the database half `migrations/0125_negative_result_class.sql`, exercised by `api/tests/memory_store_round_trip.rs::{a_negative_result_cannot_carry_a_class_the_funnel_never_produced,every_gate_refusal_produces_an_entry_the_store_accepts}` (live-Postgres, CI-run). *(Filled 2026-10-04, S-8.9.)*
- [x] AC-4: Two conflicting findings are both retrieved and marked; neither is dropped by ranking. - Verified by: `harness::memory::brief::tests::{both_sides_of_a_conflict_are_retrieved_and_marked,a_conflict_is_retrieved_whole_when_only_one_side_matches_the_goal}` and the write-side guard `harness::memory::conflict::tests::an_opposing_direction_finding_links_both_rows_and_downgrades_neither`, against Postgres `api/tests/a_finding_write_consults_the_conflict_guard.rs::an_opposing_finding_links_both_rows_and_downgrades_neither` (both rows stay `established`, both are returned by `for_subject`). `conflict::decide_write` is reached by the one door every finding passes through (`MemoryStore::put`), S-4.15. *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-5: A finding older than `valid_until` is marked `stale` in the brief and a re-verification job exists for it. - Verified by: `harness::memory::brief::tests::a_stale_finding_is_retrieved_and_flagged`, `harness::memory::retention::tests::{a_fresh_finding_is_live_and_an_expired_one_is_stale,a_stale_and_uncited_finding_archives_after_a_hundred_and_eighty_days,a_stale_finding_that_is_still_cited_does_not_archive}`, and the sweep `api/tests/a_finding_write_consults_the_conflict_guard.rs::the_retention_sweep_archives_a_stale_uncited_finding_but_the_id_still_resolves`; the monthly report is `memory_reaches_the_run::the_monthly_report_is_written_once_per_user_and_month_and_is_immutable`. **Partial:** the retention sweep is spawned from `main.rs`, but a *re-verification job* for a stale finding (as opposed to archiving it, or the continuous-research source `research-session::sources::memory::tests::{a_stale_finding_asks_whether_it_still_holds_rather_than_whether_it_holds,a_stale_finding_asks_for_a_replicate}` that queues a replicate) has no end-to-end run; S-4.16 is `PARTIAL`. Written, not re-run in this pass (needs live Postgres or a full crate build); CI runs it. *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-6: `memory_retrievals.cited` is populated; the eval suite reports recall. - Verified by: `api/tests/retrieval_recall.rs::{citing_an_entry_sets_cited_and_moves_the_share,a_run_that_retrieved_nothing_scores_none_rather_than_zero,citing_something_never_shown_moves_nothing,reassembling_the_brief_does_not_double_count}`, `harness::memory::brief::tests::{cited_is_set_when_a_later_claim_names_the_entry,a_citation_is_a_whole_retrieved_id}`, `api/tests/a_finding_that_names_a_brief_entry_cites_it.rs`, and the scorecard field `Scorecard::memory_recall`. **Partial:** `cited` is populated (server records citations, S-6.10) and recall has a computation and a column, but "the eval suite reports recall" is `None` on every published card: no live suite has run (gate H-2), and it is `None`, never `0.0`. Written, not re-run in this pass (needs live Postgres or a full crate build); CI runs it. *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-7: A playbook proposed by the agent is not resident until the verify job passes and a human admits it. - Verified by: `api/tests/skills_registry_flow.rs::{propose_verify_and_admit_with_every_refusal_naming_its_step,provisional_becomes_admitted_only_at_n_exposures_with_a_non_negative_balance,app_role_can_do_what_the_platform_does_and_nothing_that_would_bypass_admission}` and `invariants::skills_admit_is_never_an_agent_scope` (the admit scope is never granted to an agent token); the curator proposes and never writes (`harness::memory::curator::tests::its_output_is_a_proposal_and_never_a_write`). **Partial:** this proves the skills registry's propose, verify, admit path; the memory-side `playbook` entry kind reaching a human admission queue through the desk has no end-to-end test. Written, not re-run in this pass (needs live Postgres or a full crate build); CI runs it. *(Filled 2026-10-04, S-8.9.)*
- [x] AC-8: Retrieval works with the embedding sidecar stopped. - Verified by: `harness::memory::brief::tests::retrieval_works_with_the_embedding_sidecar_stopped` (retrieval is lexical-first; the sidecar is an optional re-rank), and the brief is byte-stable across two identical assemblies (`the_brief_is_byte_stable_across_two_identical_assemblies`). *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-9: An `open_question` opened in one conversation appears in the next run's brief for the same subject, `blocking` first; a finding citing it closes it with `how = answered`; `memory_retrievals.cited` is populated for it. *(Added 2026-09-14, AGENT-013 §9.)* - Verified by: `harness::memory::questions::tests::{a_blocking_question_is_not_buried_under_curiosities,a_finding_that_cites_a_question_closes_it,a_closed_question_is_not_in_the_brief,the_first_answer_is_the_one_recorded}`, the chain run against Postgres without a model `api/tests/memory_reaches_the_run.rs::an_open_question_reaches_the_next_brief_and_a_citing_finding_closes_it` (blocking first, subject-scoped, `how = answered` naming the finding, `mark_cited` logged), and the break found and fixed 2026-10-04: `harness/tests/conformance.rs::a_finding_may_cite_an_open_question_the_brief_showed`. **Partial:** that a *model* writes the citing finding is behaviour, left for a live run (S-10.12). Written, not re-run in this pass (needs live Postgres or a full crate build); CI runs it. *(Filled 2026-10-04, S-8.9.)*
## 9. Non-goals

- A general chat memory of the user ("you said you like coffee"): only `user_preference` entries with a research scope.
- Cross-user sharing of findings (all entries are per-user until a governance decision says otherwise).
- Replacing the trial ledger: memory *cites* tickets; it never duplicates them.

## 10. Open questions

- The `valid_until` policy per finding kind is a guess (90 days); AGENT-009's re-verification pass will tell us the real half-life of a market finding on this platform.
- Whether the brief should include the *user's* past questions (episodic recall across conversations) or only conclusions. Ship conclusions; measure whether the agent asks for more.
