# Phase 8 — Integration and honesty

**Completion: 67% (6 / 9 tasks; S-8.2, S-8.3 and S-8.9 partial)**

**Migration range:** the header below says 0085–0088; the set reached 0136 first. This
phase wrote `0134` (the trial family), `0135` (a battery's verdict) and `0136`
(dossiers and their comments).

> **What "6 of 9" means, and what it does not.** S-8.1, S-8.4, S-8.5, S-8.6, S-8.7 and
> S-8.8 are built and reached by a caller. S-8.2 and S-8.3 declare all six batteries
> with every step the spec names, and **one of the six can run** — the other five are
> each owed at least one tool that does not exist, and a battery with an owed step is
> absent from the catalogue rather than offered short (ADR-S8-01). That is recorded as
> partial rather than done, because a battery nobody can call is not a workflow
> anybody has.

**Requirement IDs:** TC-35…TC-40, RP-05…RP-08, EC-12.
**Specs:** [AGENT-006 §6](../../../specs/AGENT-006-tool-catalog.md),
[AGENT-008 §3.4–3.5](../../../specs/AGENT-008-reasoning-protocol.md),
[AGENT-005 §8](../../../specs/AGENT-005-context-assembly.md), AGENT-010 §10.
**Depends on:** Phases 2 (sub-runs, effect log), 3 (the tools the workflows compose), 4 (the
suites that grade them).
**Blocks:** nothing — this is the set's exit.
**Migration range:** 0085–0088.

The last phase is where the pieces stop being tools and start being a method. A workflow is
one call that runs a fixed battery with decision points and emits a verdict; a gate that
demands a workflow's ticket is a gate the model cannot argue past; a report that must carry
verified claims is a report a human can trust cold.

---

## Workflows

### S-8.1 The workflow tier — M
`DONE`

`crates/mcp-server/src/tools/workflows.rs`: a server-side sequence of typed and templated
calls with decision points, exposed as **one** tool, spending trials exactly as its steps
would, under one trial family. The model cannot tell a workflow from a typed tool — the
contract is identical.

**Files:** `crates/mcp-server/src/tools/workflows.rs` (the engine),
`tools/workflows/batteries.rs` (the declarations), `gate.rs`, `lib.rs`,
`migrations/0134_reservation_family.sql`, `crates/ledger/src/reservation.rs`,
`crates/api/src/routes/ledger.rs`.
**Acceptance:** a workflow's `ToolResult` carries every step's summary in `detail` and every
step's ticket in `ids`; its trial count equals the sum of its steps'.
**Traces:** AGENT-006 §2.2, §6; §10.6.

`Tier::Workflow` existed in the catalogue's vocabulary before this and nothing had
ever been declared with it — the same defect shape as a tool with no caller, in the
form a *category* takes.

**One battery, one family.** The id is minted **before the first step is admitted**
and every step reserves under it (`CallGate::admit_in_family`,
`dispatch_in_family`, migration `0134`), because a family-wise correction over a
denominator that does not contain a battery's own looks is a correction over the
wrong family — and it reads as a pass (ADR-S8-03). Per battery rather than per run:
a run takes many batteries, and one family per run over-corrects.

**The trial count is the sum of the steps'.** A workflow declares `TrialKind::None`
itself. Its steps are dispatched through the same table, the same gate and the same
audit envelope they would have gone through had the model called them one at a time
— which is also why "typed **and templated**" needs no special case: a templated
step reaches its arm the same way.

**Every step's ticket, keyed by the step's label** rather than by the tool's name.
`run_study` appears four times in one battery, and four ids under one key is three
ids lost.

**A failed step ends the battery only when continuing would spend.** Once a step
fails the verdict is unreachable, so every further step buys diagnostics — free from
`selection_bias_verdict`'s audit tools, and paid out of the trial budget by
`precapital_battery`'s five `run_study` steps. The rule reads the catalogue's own
`spends_trial` declaration, and the unreached steps are recorded `not_attempted`
rather than `skipped`, because "we stopped before it" and "its condition did not
apply" are different facts (ADR-S8-08).

**Evidence:** `a_battery_spends_nothing_of_its_own`,
`every_offered_batterys_steps_are_real_tools`,
`the_result_carries_every_steps_ticket`,
`a_caller_cannot_unpin_a_pinned_step_parameter`,
`a_decision_point_reads_an_earlier_step`,
`no_battery_has_a_battery_for_a_step`,
`a_failed_step_stops_a_battery_that_would_spend_to_continue`,
`stopping_early_is_not_the_same_as_not_applying`,
`building_the_catalogue_does_not_re_enter_the_catalogue`, and
`the_only_call_to_the_dispatch_table_is_behind_the_gate` (same property, one new
spelling of `admit`).

### S-8.2 The verdict workflows — L
`PARTIAL` — all three declared; `selection_bias_verdict` runs, the other two are owed tools

`leakage_audit` (provenance → static scan → bar boundary → label shuffle → perturbation →
LLM contamination if text features exist; verdict `clean | suspect | leaking`, and **"no
edge" and "leaking" are different conclusions**), `selection_bias_verdict` (N_eff →
MinTRL → DSR → PBO → family-wise), `hypothesis_precheck` (dedup → registry → negative
results, where a prior `execution_infeasible` may be retried **only** with a cited capability
delta).

**Acceptance:** each emits one verdict from a closed set with the full evidence bundle sealed
into the trial ticket; the leakage audit's bundle is permanent, not transient.
**Traces:** AGENT-006 §6; research brief Themes 1–2.

**What runs.** `selection_bias_verdict` — four calls for §6's five steps, because
`N_eff` is `trials` on the deflated-Sharpe step rather than a look of its own
(ADR-S8-04). Verdicts `passes | fails | inconclusive`; a statistic that did not come
back is `inconclusive`, never a pass.

**What is owed.** `leakage_audit` needs `scan_strategy_for_lookahead`,
`check_bar_boundary_alignment` and `llm_contamination_probe`; `hypothesis_precheck`
needs a hypothesis-registry read *by content* and a negative-results read carrying the
capability-delta rule. The steps that exist are declared and wired, so each battery is
one tool from running.

**Bundle permanence** is `mlops.workflow_verdict` (`0135`): every step, its status and
its ids, written by the engine from steps it dispatched — not by a tool the model
calls, which is what lets a gate read it (S-8.4).

**Evidence:** `no_edge_and_leaking_are_different_conclusions`,
`clean_does_not_claim_an_edge`, `a_flagged_bar_is_suspect_and_not_leaking`,
`each_correction_can_fail_the_verdict_alone`,
`a_missing_statistic_is_inconclusive_and_not_a_pass`,
`a_verdict_with_no_steps_is_not_a_verdict`.

### S-8.3 The capital workflows — L
`PARTIAL` — all three declared; none runs yet (ten owed stages between them)

`precapital_battery` (ablation → fragility → cost shock → reshuffle → jitter → regime →
synthetic stress on bootstrap/fitted generators **only** → red-team falsify), `signal_identity`
(half-life → spanning → style → cs/ts → health check → crowding), `go_live_readiness`
(capacity → venue constraints → funding → divergence → shadow session).

**Acceptance:** `precapital_battery` reaches `reject | reduced_size_paper | full_paper`;
a GAN/diffusion generator can never contribute to a pass verdict.
**Traces:** AGENT-006 §6; research brief Themes 3–5.

`precapital_battery` is seven of eight steps: ablation, fragility, cost shock,
reshuffle, regime and synthetic stress are `run_study` kinds, and the red-team pass is
`run_oracle_test`. It is owed **timing jitter**, which cannot be a `run_study` kind at
all because `VarySpec` has no execution-delay dimension — the simulator has to gain the
knob first.

`signal_identity` is owed all six of its steps and `go_live_readiness` four of its
five; both are declared with what each step needs. `go_live_readiness`'s crowding step
may be owed for a long time, because free-only data (BS-007) has no positioning feed —
which is worth stating rather than quietly dropping.

**The generator rule is a list, not a threshold.** `PASSING_GENERATORS` is the set
whose paths can license capital: bootstrap and fitted parametric. A verdict function
reading only p-values would let a GAN pass on a small number, so this reads the
generator's *name* and refuses anything outside the list, including an unnamed one.

**Evidence:** `a_learned_generator_cannot_license_capital` (four learned generator
names, and every permitted one accepted),
`an_unnamed_generator_is_not_a_permitted_one`,
`the_hard_checks_reject_and_the_soft_ones_reduce_size`,
`a_strategy_that_survives_the_whole_battery_reaches_full_paper`.

### S-8.4 Gates demand tickets — M
`DONE`

Amend the funnel: Gate 2 requires `leakage_audit = clean` **and**
`selection_bias_verdict = passes`; Gate 3 requires `precapital_battery ≠ reject`; the paper
gate requires `go_live_readiness ≠ no_go`. The gate refuses to advance without the ticket ids.

**Files:** `crates/backtest/src/gates/tickets.rs`, `crates/api/src/routes/experiments.rs`,
`crates/api/src/routes/ledger.rs`, `migrations/0135_workflow_verdict.sql`.
**Acceptance:** `advance_gate` to Gate 2 fails without both tickets, and the failure names
which is missing. Adversarial: a persuasive argument with no ticket is refused.
**Traces:** AGENT-006 §6; AGENT-008 P8.

The requirement is a **table**, because the same four questions are asked at three
checkpoints and one of them is not a gate — `go_live_readiness` belongs at the paper
door (ADR-S8-07). `advance_funnel` checks gate 2's and gate 3's requirements *before a
single Study runs*: the alternative is minutes of CPCV to reach a refusal a table
lookup already knew.

**A persuasive argument is refused because there is nowhere to put one.** `check` reads
rows from `mlops.workflow_verdict`, which only the workflow engine writes; the request
being judged carries no justification field at all; and a ticket lookup that *failed*
refuses rather than advancing.

**Three of the four requirements are declared and not enforced**, because their
batteries cannot run (ADR-S8-02). `enforced` is a `const` held equal to
`Workflow::is_runnable()` by `crates/invariants/tests/gates_demand_tickets.rs`. No
configuration, environment variable or request field reaches it, and a battery
becoming runnable fails that test until the gate starts demanding it.

**Evidence:** `a_checkpoint_refuses_without_its_ticket_and_says_which`,
`a_persuasive_argument_with_no_ticket_is_refused`,
`a_failing_battery_is_not_a_missing_one`, `inconclusive_does_not_advance_a_gate`,
`the_newest_ticket_wins_and_not_the_best_one`,
`a_requirement_is_enforced_exactly_when_its_battery_can_run`.

---

## The report layer

### S-8.5 Claims, and rechecking them — M
`DONE`

`report.extract_claims` decomposes agent output into `{claim, evidence_refs, confidence,
verification_method}` by **post-hoc span alignment**, not by trusting the model to self-cite —
LLMs find relevant documents well and pinpoint the supporting span badly.
`direct_stat` is auto-assignable only when a number in the claim exactly matches a value in
the cited evidence; anything else is `derived` or `unverified`.
`report.recheck_claim` re-derives via a real tool call, and a failed recheck **blocks** the
report rather than logging.

**Files:** `crates/harness/src/report/claims.rs`, `crates/api/src/dossier.rs`,
`crates/api/src/routes/dossiers.rs`.
**Acceptance:** an unsupported claim blocks finalisation; a claim whose number does not match
its evidence cannot be `direct_stat`.
**Traces:** AGENT-006 §5.8; AGENT-008 §3.4 rungs 6–7; research brief Theme 9.

Three methods, and the middle one is load-bearing: a number the evidence **rounds to**,
at the precision the claim states, is `derived` and does not block. Without
`is_rounding_of` the block would fire on almost every report, because reports round —
and a check that always fires is a check people turn off. The relation is deliberately
asymmetric: stating fewer digits is a summary, stating more is a fabrication.

`recheck_claim` is `POST /api/dossiers/{ticket}/verify/{claim}`, and it **re-derives**
rather than re-reading: it re-renders the evidence from the platform as it stands now
and re-runs the alignment (ADR-S8-05).

**Where "a failed recheck blocks" lands.** At compile: an `Unverified` claim stops the
dossier being written at all, which is the only moment a block can mean anything. The
live `verify` runs against a document that already exists and is immutable by design,
so it cannot block — it can only *disagree*, which is what it is for. Reading the
acceptance the other way would mean a dossier that rewrites itself when its evidence
moves, and a document that changes after the decision it justified is a document
nobody can cite.

**Stated limitation:** a claim is a sentence containing a number, so a qualitative
overclaim with no number in it is not caught here. That is the checklist critic's
`SuggestiveNotPresentedAsEstablished` check, which reads the platform's own confidence
label rather than the prose.

**Stated requirement on the caller:** the pre-registration must be in the evidence
bundle, or every comparison to a committed threshold blocks as an invention.
`api::dossier::registration` is what honours it.

**Evidence:** `a_rounded_number_is_derived_and_not_direct`,
`stating_more_digits_than_the_evidence_is_not_a_rounding`,
`a_pair_is_direct_only_when_both_appear`, `a_report_blocks_on_its_worst_claim`,
`rendering_puts_every_number_where_alignment_can_find_it`,
`a_recheck_fails_only_when_the_claim_got_weaker`.

### S-8.6 The critic, in a fresh session — M
`DONE`

`report.adversarial_critique` runs as a sub-run (S-2.12) with a **fresh context**, the
evidence bundle only, read-only tools, and never the producer's reasoning trace — separated
*sessions*, not just separated prompts, because same-context self-critique measurably
underperforms. Smaller tiers get a deterministic checklist critic instead: every acceptance
criterion has an evidence id, every trial is in a family, no `suggestive` finding is
presented as established, the null comparison ran. Track reviewer precision over time.

**Acceptance:** the critic cannot read the producer's transcript, asserted structurally; a
`blocking` flag sends the draft back rather than annotating it.
**Traces:** AGENT-008 §3.4 rung 8; AGENT-006 §5.8; research brief Theme 3 (naive debate
underperforms).

The structural assertion is two mechanisms and neither is an instruction to a model:
`Projection::Critic` leaves the producer's reasoning out of the store the critic
selects from, and `Delegation::check` refuses a critic dispatched under any other
projection.

**Not tracked yet:** reviewer precision over time. It needs a corpus of critiques whose
findings were later confirmed or dismissed, and nothing records the second half — see
the complacency entry.

**Evidence:** `the_model_critic_cannot_read_the_producers_reasoning`,
`every_failure_is_reported_not_just_the_first`, `an_empty_draft_is_not_a_clean_one`, and the four checks' own blocking tests.

### S-8.7 The dossier — M
`DONE`

`report.compile_dossier` with mandatory "Unresolved questions" and "Reviewer critique"
sections that must say "none found" **explicitly** rather than being omitted — an omitted
section reads as suspicious to an experienced reviewer. Plus the API the desk reader needs:
`GET /api/dossiers/{ticket}`, `…/verify/{claim}`, `…/comments`.

**Files:** `crates/harness/src/report/dossier.rs`, `crates/api/src/dossier.rs`,
`crates/api/src/routes/dossiers.rs`, `migrations/0136_dossiers.sql` (planned as `0085`),
`frontend/src/components/desk/Dossier.tsx`.
**Acceptance:** a dossier with nothing unresolved still renders the section with "none found";
`verify` re-runs a claim's recheck live.
**Traces:** AGENT-006 §5.8; AGENT-012 §5.

`Parts.critique` is `Option<Vec<String>>` so that "no critic ran" (`None`, refused) and
"the critic found nothing" (`Some(vec![])`, renders "none found") cannot collapse into
one value — the mistake the whole module is about, expressed in the type.

**This closed a caller with no callee.** `frontend/.../Dossier.tsx` has called
`/api/dossiers/{ticket}` since S-6.17 and rendered an empty state naming a route that
did not exist. The comment thread is new on both sides at once, so it does not repeat
the trick.

**Compiled where?** `Recorder::finish`, for a run that produced a result. A fenced run
has no report whose claims could be checked, and compiling one would be inventing a
document about a run that did not finish.

**Evidence:** `an_empty_section_still_renders_and_says_so`,
`each_empty_section_says_what_was_empty`,
`a_critique_that_never_ran_is_not_an_empty_critique`,
`an_unsupported_claim_blocks_compilation`, `a_dossier_names_the_trial_it_rests_on`,
`the_verification_words_are_the_ones_the_desk_knows`.

---

## Code, and the close

### S-8.8 `run_python` and the container's new job — L
`DONE`

One tool, namespace `code`, risk `write`, gated by the existing
`orchestration.code_execution` field (`profile.rs:164`, already enforced at `:398`):
`denied`/`templated` tiers run templates on the host skill runner; `full` runs the
**container** from ADR-0024, repurposed — `infra/agent-image` keeps its research stack,
non-root user, read-only root and internal network, **drops the Claude Agent SDK**, and runs
a small executor accepting `{script, payload, data_path}`. Started per conversation, mounting
that conversation's workspace, torn down with it. **Build the image in CI** — nothing builds
it today — and add an `agent-sandbox` compose service.

**Files:** `infra/agent-image/Dockerfile`, `infra/agent-image/executor.py`,
`.github/workflows/ci.yml`, `docker-compose.yml`,
`crates/mcp-server/src/tools/code.rs`, `crates/api/src/routes/code.rs`,
`crates/harness/src/registry.rs`.
**Acceptance:** `run_python` on the `full` tier executes in the sandbox with the
conversation's workspace mounted and **no provider credential present**; on a `denied` tier
the tool is not exposed while the templated tools still are.
**Traces:** AGENT-005 §8; AGENT-006 §5.9; AGENT-010 §10; ADR-0035.

The image dropped the Claude Agent SDK, `agent-host` and the platform client, and its
entrypoint is `executor.py` — one request on stdin, one response on stdout.
`executor.py` **refuses to start** if it finds any of ten credential environment
variables, refuses to run as root, and resolves every request path inside the workspace
before checking it.

CI builds the image and checks all three: a script runs and returns its result; a
container holding `ANTHROPIC_API_KEY` refuses to start; a `data_path` of `../etc/passwd`
is refused.

The exposure gate is the whole `code` namespace, in `ToolRegistry::expose_within`, so a
model below `full` never sees the tool rather than being refused after spending a step.
`crates/invariants/tests/code_execution_is_tiered.rs` holds the two crates' namespace
constants equal and checks all three shipped profiles.

**What is a deployment contract and is not claimed as built:** the per-conversation
container *launcher*. `POST /api/code/run` creates the workspace and shells
`TBOT_SANDBOX_CMD` with `{workspace}` substituted; nothing here starts or tears down a
container, and with the variable unset the tool refuses rather than falling back to the
host (ADR-S8-06).

**Evidence:** the three CI steps above,
`the_sandbox_cannot_run_the_simulator_so_it_spends_no_trial`,
`free_form_code_is_not_a_read`, `only_a_full_tier_is_offered_the_code_tool`,
`a_workspace_path_cannot_come_off_the_wire`.

### S-8.9 Close the set — M
`PARTIAL`

Run every suite; publish the scorecards; fill the last `Verified by:` slots across the nine
specs; update `docs/parity-matrix.md` with the new behaviours and their evidence; move the
nine specs from `Proposed` to `Implemented` in `docs/specs/README.md`; mark AGENT-001…004 and
COMP-006 superseded; record in `PROGRESS.md` what is enforced by mechanism and, honestly,
what is not.

**Acceptance:** all 75 acceptance criteria carry evidence or a recorded, justified exception;
`COMPLACENCY_LOG.md` has an entry for every exception with the phase that would close it.
**Traces:** `docs/procedures/execute-plan.md` §3c; MASTER §6.

**Done.**

* `docs/governance/COMPLACENCY_LOG.md` gained a Set S phase 8 section: **eight** recorded
  exceptions, each naming what would close it. Kept separate from the 2026-06 repository
  scan above it, because that list asks "what is wrong" and this one asks "what did we
  ship knowing it was incomplete" — different questions, and merging them would bury the
  second.
* `docs/parity-matrix.md` gained a section for the nine behaviours with **no Python
  counterpart**, each with the test that proves it. Framed that way rather than as parity
  rows, because the Python system had none of them and an empty "Python status" column
  would read as a regression.
* `docs/ml-platform-context-pack/decisions/PROGRESS.md` records what is enforced by
  mechanism and, separately, what is not true.
* Seven ADR entries (`ADR-S8-01` … `ADR-S8-07`) for the judgement calls this phase made.
* `Verified by:` filled on AGENT-005 AC-11 (new), AGENT-006 AC-6 and AC-8, and AGENT-008
  AC-4 — three of the four marked **partial**, with the unproved half named.
* AGENT-001, AGENT-002, AGENT-003, AGENT-004 and COMP-006 are marked **superseded**, each
  naming its replacement and why it is kept rather than deleted.

**Not done, and why.**

* **The scorecards are not published.** `harness::eval::scorecard::Scorecard` exists and
  its fields are honest about this already: `cache_hit_rate` is documented as `None` until
  a live session has run, which needs a model credential the platform does not have. This
  is roadmap gate **H-2**, not a phase-8 omission, and publishing a card whose live
  suites are empty would be publishing a measurement of nothing.
* **The nine specs are not moved from `Proposed` to `Implemented` in
  `docs/specs/README.MD`.** Two reasons, and the second is the binding one. The table in
  that file has no status column for these specs — it lists file, scope, ADRs and plan set
  — so there is no `Proposed` to move. And the file is under a standing instruction not to
  be modified or staged in this repository. The status that *does* exist is the one on each
  spec's own header, which is where the five supersessions above were written.
* **`Verified by:` is not filled across all nine specs**, only where phase 8 produced the
  evidence. Four of the nine (AGENT-007, AGENT-009, AGENT-010, AGENT-012) carry no
  `Verified by:` slots at all, and filling the rest would mean claiming evidence from
  phases that have not run.

---

## Exit criteria

| | Criterion | Status |
|---|---|---|
| 1 | A battery is one call, one trial family, one verdict. | **Met for the mechanism, and for one battery.** The engine, the family (`0134`) and the closed verdict sets are built and tested; five of the six batteries cannot be called because they are owed tools. |
| 2 | A gate refuses to advance without the tickets that license it. | **Met for the mechanism; one of four requirements is live.** The refusal reads rows only the engine writes, is checked before any Study runs, and the three unenforced requirements are held to their batteries' runnability by an invariant rather than by a switch. |
| 3 | A report cannot be finalised with an unsupported claim, and the critic that checks it cannot see the reasoning that produced it. | **Met.** Alignment blocks the dossier; the critic's isolation is two mechanisms, neither of them an instruction to a model. |
| 4 | `run_python` exists at the `full` tier, in a sandbox the agent's judgement does not run in, and the image is built by CI. | **Met**, with the per-conversation *launcher* a deployment contract rather than a component here — and the platform refusing rather than falling back to the host when none is configured. |
| 5 | Every one of the 75 criteria is closed with evidence, or its exception is recorded where the next person will find it. | **Met for this phase's criteria.** Eight exceptions in `COMPLACENCY_LOG.md`, each with what closes it. The set-wide count is S-8.9's remaining work and depends on phases that have not run. |

**What a reader should take from the first two rows.** The mechanisms are real and
adversarially tested; what is thin is the *supply* of batteries, and that is sixteen
named tools rather than a design problem. Nothing here is enforced-by-comment: the
unenforced gate requirements fail a test the moment their battery becomes runnable.
