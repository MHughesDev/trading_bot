# Phase 10 — Modes and the research lifecycle

**Completion: 31% (5 / 16 tasks), plus 11 `PARTIAL`. One untouched: S-10.15.**

> Status 2026-09-17. **S-10.1, S-10.6, S-10.7 and S-10.13 are done**; S-10.11 is
> partial and substantial — the `ResearchRun` worker, the campaign door, the worker
> projection and the `propose` decision are built and reached, and what is missing
> is the parent's resumption, which waits on `Effect::Await` resumption in the
> driver. S-10.14's per-tier half landed with it. Only S-10.15 is untouched.
>
> What was already here and doing nothing: `WorkingState` has carried `mode`,
> `workflow`, `ring` and `stage` since phase 1 and the state card renders all four;
> `ToolSpec::ring` has derived the ring from risk and envelope, with its own tests.
> **Nothing filtered on any of it.** A filter is worth exactly what the field it reads
> is worth, and this one was reading nothing — the eighth instance of this repository's
> dominant defect shape.

**Requirement IDs:** LC-01…LC-16.
**Spec:** [AGENT-013](../../../specs/AGENT-013-modes-and-research-lifecycle.md); amendments
it makes to AGENT-006 §2.1/§5.11, AGENT-007 §2.2/§4, AGENT-008 §2/§4, AGENT-009 §3,
AGENT-010 §4/§9, AGENT-012 §2/§3; [ADR-0037](../../../adr/0037-two-modes-continuous-toggle.md).
**Depends on:** Phase 1 (`WorkingState`, `ToolSpec`), Phase 2 (one loop, effect log, sub-runs),
Phase 4 (verified findings, negative results, the brief, trajectories), Phase 8 (workflows,
gates demanding tickets, the dossier). S-10.15 depends on Phase 6.
**Blocks:** Phase 11 (continuous sessions run Research runs and read their yield).
**Migration range:** 0089–0091.
**Parallel with:** Phase 9.

This phase turns the harness Set S builds into a *research process*. Set S makes a step
honest and a run durable; nothing in it says what a run must pass through before its answer
counts. Today a run can choose its evaluation after seeing a result, refine a strategy without
knowing why it failed, report `inconclusive` because nothing was ever written that could have
been refuted, and end having produced no knowledge while every dashboard shows activity. The
sixteen tasks here make the lifecycle (intake → ground → preregister → hypothesise → design →
execute → evaluate → diagnose → refine → report) enforced rather than advised, define the two
modes the user chooses between (and the Continuous toggle on Research, AGENT-014), state the four rings of what the agent may modify, and make
every run report what it produced as a number.

**The load-bearing idea.** The trial ledger makes every look *counted* (INV-16).
Pre-registration makes every look *pre-committed*. The platform already does the second per
Experiment (`prereg_hash`); S-10.3 extends it to the run and adds the falsifier, which is the
largest door still open.

---

## Modes and rings

### S-10.1 Mode, workflow, ring and stage on the run; the ring filter on exposure — M
`DONE` 2026-09-16 — `registry::ring_of` + `registry::Limits`, filtering in
`expose_within` **before** the budget and in `search_tools`; migration `0126` with
CHECK lists mirrored against the Rust enums by `schema_mirrors_lifecycle.rs`.

**One departure, recorded as `ADR-S10-01`:** a Research run starts at ring 1, not
ring 0. §4 wants ring 0, widened by a proposal a human decides — and that door is
S-10.9 and does not exist. Shipping the narrow default first would leave every
Research run unable to create a strategy. A Chat turn *is* ring 0 and cannot be
widened, which is the half that can ship today.

`agent_runs` gains `mode: chat|research`, `workflow` (the seven names), `ring: 0..2`, `stage`,
`session_id` (AGENT-014 §10 — every Research run has a session row; a continuous session's
children carry its id); `WorkingState` (Phase 1) gains `stage`, `mode`, `workflow`, `ring` and renders them
in the card's first line. The **ring filter** runs before routing (AGENT-005 §5): the
catalogue slice a run can route over is filtered by ring first, so a tool outside the ring is
unnameable, not refused. Ring membership is a `ToolSpec` field derived from risk and target
(`registry`/`book`/`monitor` writes → ring ≥ 1); a build-time test asserts every write tool
has one.

**Files:** `crates/harness/src/state.rs`, `crates/harness/src/registry.rs` (filter before
`search`), `crates/mcp-server/src/tools/spec.rs`, migration 0089.
**Acceptance:** a Chat run's slice contains no tool with `spends_trial ≠ None` and no ring-1/2
tool; `search_tools` from a Chat run cannot surface `run_study`; the run row and the state card
carry mode, workflow, ring, stage; conformance reads every new field.
**Traces:** AGENT-013 §2.1, §4, §5; §13.1. ADR-0037 §6.

### S-10.2 Chat: `Lookup` forced, no ticket, and the Research offer — M
`PARTIAL` 2026-09-16 — **the refusal is built; the offer is not.**

Done: `CallGate::for_chat_turn` refuses every trial-spending tool at the door,
before the registrar, so no row is written and no budget is consumed; the code is
`chat_turn_spends_no_trials` and it is routable, because the desk has to turn it
into an action. That is the belt to S-10.1's braces, and the two are kept separate
deliberately — exposure is a routing property a later code path could route around,
and this is a refusal every handler goes through.

**Not done, and this is why the conversation front door still runs as `research`:**
the Research offer card, its *Start* action, and the `user_preference` that skips it.
Declaring the front door `Chat` today would make it ring 0 and lookups-only with
nothing offered in place of what it lost — a capability removed and no door built.
The mode and the ring are derived from one input in `manager.rs` so the run row and
the loop cannot disagree meanwhile; flipping the front door is one constant once the
offer exists.

A Chat turn forces `GoalKind::Lookup` (AGENT-008 §3.1), a plan of ≤ 3 read-only tools, hides
`report.*`. When the classifier rates the goal `Research|Diagnose|Compare`, or the model asks
for a trial-spending tool, the turn ends with a **Research offer** card (new card type: the
question as understood, the workflow, estimated cost and trials, the pre-registration it would
seal, one *Start* action). The ticket path (`crates/ledger`) refuses any ticket request whose
run has `mode = chat` — belt and braces with S-10.1, because exposure is a routing property and
the ledger is a ledger property. A `user_preference` "always start" per scope skips the offer.

**Files:** `crates/harness/src/drive.rs`, `crates/ledger/src/ticket.rs`,
`crates/api/src/agent/manager.rs`, `frontend/src/components/desk/cards/ResearchOffer.tsx`.
**Acceptance:** a Chat goal classified `Research` produces the offer and spends zero trials;
*Start* opens a Research run with the same question and constraints; a forged ticket request
with `mode = chat` is refused (adversarial test); the offer is skipped under the preference.
**Traces:** AGENT-013 §2.2; §13.1, §13.2.

## Pre-registration, hypotheses, verdicts

### S-10.3 `Preregistration` and the ledger gate — L
`PARTIAL` 2026-09-16 — **the object, the hash, the lineage and the gate are built;
the tool that seals one is not.**

Done: `crates/ledger/src/prereg.rs` — every field required with no serde default
(ADR-P0-08's technique, so a body without `delta_practical` fails to deserialise
rather than arriving as zero); the embargo computed through
`dataplane::split::compute_embargo` so a run's embargo and its splits cannot
disagree; **the falsifier in the hash**, which is the extension §7.2 makes over the
per-Experiment `prereg_hash` and the door the whole object exists to close; amendment
lineage with the trial budget carried over, so amending cannot buy a fresh budget;
the third amendment refused as `protocol_unstable`. Migration `0127` repeats each
rule where a second writer cannot route around it, and adds one the Rust cannot: a
unique root per run, so a run cannot seal a second convenient protocol beside the
first instead of amending it.

`CallGate::under_preregistration` refuses every trial-spending tool on a run with
`Prereg::Missing`, before the registrar, and stamps the hash on every ticket a
sealed run spends. `Prereg` is a three-state enum rather than an `Option` because
"not required" and "required and missing" must not be the same value.

`lifecycle.preregister` and `lifecycle.hypothesise` exist, with their routes
(`POST /api/lifecycle/preregistrations`, `/hypotheses`, `GET /metrics`) and their
authoring-guide notes. Amending reads the row it supersedes, so the trial budget is
carried by the ledger rather than trusted from the request.

The driver reads the head of the run's amendment chain and builds the gate from it,
so a Research run that has sealed nothing is refused and one that has sealed
something stamps the hash on every ticket. A read that *fails* is `Missing`, not
`NotRequired`: treating a database error as "no registration needed" would make an
outage open the gate, and the gate is what stands between a run and the trial
budget.

**The two profile values landed 2026-09-17.** `max_prereg_amendments` is declared,
validated (never above `ledger::prereg::MAX_PREREG_AMENDMENTS`, never zero), read
by `POST /api/lifecycle/preregistrations` when it amends, and held equal to the
ledger's ceiling by a cross-crate invariant — the two constants are written
independently because `harness` is sans-IO and importing `ledger` for one integer
would drag sqlx into it. The shipped profiles narrow it with the tier: three on
frontier, two on `local_mid`, one on the degraded tier, which matches its
refinement cap.

**`prereg_authoring` landed with the thing it selects**, because a profile value
whose enforcement point does not exist is the bug this phase keeps finding.
`harness/src/lifecycle/skeleton.rs` builds the form §11's two constrained modes are
handed, and `local_driver` attaches it to the `preregister` call — the same
argument-completion site `define_campaign` uses, and for the same reason: the model
cannot supply what the platform owns.

What the skeleton fills is what the *question* already fixed — the window, the
instrument the objective is about, the gate profile, the null family. What it
refuses to fill is the point: `primary_hypothesis` and `falsifier` are the claim,
and a skeleton that guessed them would be the platform pre-registering on the
model's behalf and the model agreeing, which is a commitment by nobody.
`delta_practical` has no platform default by design and a skeleton is a *worse*
place for one than code, because it would arrive looking computed. **The model's own
values always win** — `or_insert`, never overwrite — or "confirm this" would mean
"your answer was discarded".

Two further holes are named rather than filled, and both are findings rather than
principles: **there is no cost-model registry** to name a `cost_model_version` from
(the run config carries a `cost_model_ref` string and nothing resolves it), and **no
profile declares a per-tier trial cap**, so there is no `trial_budget` the platform
owns. Writing either would pin every result the run produces to a number nobody
published. `Skeleton::what_you_must_write` names all five holes with why each is the
run's own.

**Not done, and it is the acceptance's own sentence:** `run_study` refusing is
asserted at the gate, not end to end — that needs a live run (gate H-2).

**`preregistration_required` is deliberately not built.** S-10.3's text makes it a
profile value; §11's own table gives every tier a pre-registration *authoring mode*
and has no row meaning "none", so a boolean that switched it off would be a fourth
mode the table does not have, available at every tier. INV-16 would not stop it
either — `mlops.trial.prereg_hash` predates AGENT-013 and a per-Experiment hash
satisfies the constraint while carrying no falsifier at all. Written up as **OQ-19**
with three options rather than implemented in either direction. What holds meanwhile
is stricter than either reading: a lookup needs none, every Research run does.

The object (AGENT-013 §7.2): objective in SPEC §10's shape, benchmark, `delta_practical`
(no serde default — a request without it fails to deserialise, ADR-P0-08's technique), window,
computed embargo (SPEC §12.2), cost model version, null family, trial budget, gate profile,
primary hypothesis, falsifier, hash, ledger ref. `lifecycle.preregister` seals it; the ledger
row attests it predates the trials. **The gate:** the ticket path refuses a request carrying no
`prereg_id` for the run with `preregistration_required`; every ticket carries the hash.
Amending creates a new row with `supersedes`; the trial count carries over; the third amendment
ends the run `blocked: protocol_unstable` (`max_prereg_amendments`, profile value, reader).

**Files:** `crates/ledger/src/prereg.rs` (new), `crates/ledger/src/ticket.rs`,
`crates/mcp-server/src/tools/lifecycle.rs` (new namespace), migration 0089.
**Acceptance:** `run_study` on a run with no sealed registration is refused with the named
reason; a registration without `delta_practical` fails at the boundary; amendment lineage and
the carry-over are asserted; the third amendment blocks the run; the hash is on every ticket
the run spends.
**Traces:** AGENT-013 P2, §5.3, §6.1, §7.2, §8; §13.3, §13.4.

### S-10.4 Hypotheses name their falsifier; the platform writes the verdict — L
`PARTIAL` 2026-09-16 — **the falsifier is validated and the verdict is unwritable;
`evaluate` itself is not built.**

Done: `lifecycle::check_hypothesis` refuses a falsifier that names no metric in the
closed vocabulary, and one that names a metric with no comparator — a metric alone
is a noun, not a claim: `sharpe_after_costs` cannot turn out false and
`sharpe_after_costs <= 0` can. The role is a closed list. `0127`'s
`agent_lifecycle.hypothesis` has `status` and `inconclusive_why` with a CHECK pair
that makes `inconclusive` say *why* and nothing else carry a reason.

**The request type has no status field at all**, which is §6.2's mechanism rather
than an omission — there is nothing to ignore carefully — and a test asserts it.

`harness/src/lifecycle/evaluate.rs` is built and is a pure function of the
tickets: it parses the falsifier out of the prose a model actually writes, decides
`supported | refuted | inconclusive{why}`, and every `inconclusive` carries a reason
from the closed list. **`supported` needs §6.9's whole battery** — out-of-sample,
cost-model and benchmark tickets plus a reproducibility receipt — and a refutation
needs none of it, deliberately: requiring the battery before admitting a failure
would make failing expensive and passing cheap, which is backwards for a platform
whose main output is meant to be negative results.

`record_finding` is corrected when it grades its own hypothesis, and the check is
narrow on purpose — a model corrected for writing English learns to write worse
English rather than to stop claiming verdicts.

`schema_mirrors_lifecycle.rs` holds the reasons, the statuses and the roles equal to
the Rust enums. `open` has no Rust variant on purpose: it is the absence of a
verdict, and a `Verdict::Open` would be a value `evaluate` could return.

**Not done:** nothing *calls* `evaluate` — the funnel's return has no hook into it,
so every hypothesis stays `open`. That needs the funnel and a live run (gate H-2).
The metric vocabulary is also a constant in `routes/lifecycle.rs` rather than a read
of `mlops.trial_statistic`: a copy, and marked as one.

`lifecycle.hypothesise` validates the falsifier against the closed metric vocabulary
(`mlops.trial_statistic`'s) and a comparator; a falsifier naming nothing evaluable is refused
with the missing part named. `role: primary|secondary|exploratory`; secondary/exploratory
results cap at `suggestive` (AGENT-007 §3.1 step 2 extended). **`evaluate` is a platform
function**, not a tool: when the funnel returns and the metric is in `trial_statistic`, it
writes `Hypothesis.status` and `Verdict` (`supported|refuted|inconclusive{why}`); `why` is one
of `insufficient_data|protocol_unstable|evidence_conflict|budget_exhausted|evidence_incomplete`.
A `record_finding` that tries to set status is corrected.

**Files:** `crates/mcp-server/src/tools/lifecycle.rs`, `crates/harness/src/lifecycle/evaluate.rs`
(new), `crates/harness/src/drive.rs` (finding path), migration 0090.
**Acceptance:** a falsifier with no evaluable metric is refused; a sealed falsifier's status is
written by `evaluate` from real ticket numbers on the reference dataset; the model's attempt to
set it is corrected; an exploratory hypothesis cannot yield `established`; every `inconclusive`
carries a `why`.
**Traces:** AGENT-013 P3, §5.4, §5.7, §6.2, §6.6, §6.7, §7.3, §8.1; §13.5.

### S-10.5 Design, the power check, and `simplify` before the first trial — M
`PARTIAL` 2026-09-17 — **the power check is built and binding; `simplify` is not.**

`crates/research/src/power.rs` asks `min_track_record_length` about the
*pre-registered effect size* rather than about an observed Sharpe, which turns it
from a verdict into a precondition. A run that registers a `delta_practical` and
then looks for it in a window that could not show it spends every trial it has and
learns that at the end; this costs nothing and says so at the start.

**One-sided on purpose.** `Adequate` is a floor — "this run is not doomed on sample
size alone" — and the refusal explicitly is not a promise the window will settle the
question. A check that read as a guarantee would be wrong the first time the returns
were autocorrelated, and it says so in its own `detail()`.

The refusal names the window that would suffice. "Insufficient data" is not
something a run can act on; "it needs 1,340 observations, has 512, and at this
sampling rate that is 5.3 years" is.

**Neither the delta nor the window is in the design request** — both come from the
sealed registration, and a test asserts the request type cannot carry either. The
observation count is derived from the window and the bar size for the same reason: a
run that could state its own would state the one that passes.

`lifecycle.design` refuses `strategy_version: latest` at the boundary — a design
bound to it produces a result nobody can reproduce a week later — and `0128`'s
`a_design_names_an_exact_version` refuses it again where a second writer cannot
route around it.

**Not done:** `simplify()` over the draft, and the `Move` diff it records. The
observation count also assumes 24/7 (crypto, which is what the platform's live
venues are); an equities window would need a trading calendar, and that limitation
is named in `observations_in`'s doc rather than left to be discovered.

`lifecycle.design` binds a hypothesis to a strategy version, split spec and null choice, and
runs the **power check**: `min_track_record` (AGENT-006 §5.4) for the pre-registered
`delta_practical` against the window; insufficient → refused *before any trial*, with the
window that would suffice. `simplify()` (FEAT-003 §8.3) runs on the draft; a rule whose
removal does not reduce the objective in `preview_signals` is removed and the diff is recorded
as a `Move`.

**Files:** `crates/mcp-server/src/tools/lifecycle.rs`, `crates/research/src/power.rs`.
**Acceptance:** a design under the minimum track record is refused with the sufficient window
stated and zero trials spent; `simplify` removes a planted redundant rule and records the move;
the design id is on every trial that follows.
**Traces:** AGENT-013 §5.5, §6.3; §13.6.

### S-10.6 Refinement chains, and when they stop — M
`DONE` 2026-09-17 — `harness/src/lifecycle/chain.rs`. All four stops, checked in
the order the reasons invalidate each other: an unstable protocol makes the numbers
meaningless, so it comes before any rule that reads one.

The refusal names **both** required fields even when only one is missing — a model
told about one requirement supplies it and is refused again, and two refusals for
one request teach it the tool is unreliable rather than that the request was.

The same class *twice consecutively*, which is not the same as twice in the chain: a
run that hit costs, fixed them, regressed and hit them again has learned something
between the two.

`Prospect::NotComputable` below three links, and it hands the decision to the cap
rather than guessing — ADR-P2-04's discipline, and the reason is that an
approximation here reads as a measurement and would stop a chain that was working.

`max_refinements_per_run` is a profile value (3 frontier, 2 local) and conformance
reads it **behaviourally**: it builds a chain at the cap and asserts the cap is what
stops it. A range assertion would have passed while nothing read the field.

`retry_condition` delegates to `FailureClass::retry_condition` rather than
re-deciding, so the `None` half survives: a weak effect has no retry condition, and
offering one anyway is the relabelling `negative.rs` exists to stop.

`lifecycle.refine` requires `diagnosis_id` and `predicted_effect`; missing either → refused
with both named. Chains are tracked from the original design. Stop rules: `max_refinements_per_run`
(3 / 2 local, profile value); **diminishing returns** — SPEC §10's posterior rule at run scale
(`P(next > incumbent by delta_practical) < 0.05` over the chain; `NotComputable` below three
and the count governs); the same failure class twice consecutively; a third pre-registration
amendment. Stopping produces the negative result with `retry_condition` filled from the last
diagnosis.

**Files:** `crates/mcp-server/src/tools/lifecycle.rs`, `crates/harness/src/lifecycle/chain.rs`.
**Acceptance:** refinement without diagnosis or prediction is refused; the same class twice
ends the chain; the cap is read by conformance; `NotComputable` is reported, never approximated
(the ADR-P2-04 discipline); the resulting negative result carries a `retry_condition`.
**Traces:** AGENT-013 P4, §5.9, §6.5, §7.4; §13.7.

### S-10.7 Failure classes, extended and derived — S
`DONE` 2026-09-17 — the four classes were already in the enum and in `0125`'s CHECK;
what was missing was the **derivation**. `harness/src/lifecycle/diagnose.rs` maps
each to exactly one producer — `leakage_audit`, the amendment cap,
`parameter_fragility_map`, `regime_failure_search` — and the order encodes what
invalidates what: leakage first, because a run told "costs dominate" about a leaking
backtest has been told something false and will go and work on costs.

A diagnostic **nobody ran** is reported separately from one that came back clear;
collapsing them would let "we did not look" masquerade as "we looked and it was
fine". A contradiction with memory travels beside the class rather than resolving
into it — P5, in the fourth place it comes up.

`the_agent_cannot_name_a_class` scans the module's own non-test half: no function
here takes a class as an argument.

AGENT-007 §3.2's enum gains `leakage`, `protocol_unstable`, `fragile_parameters`,
`regime_specific`, each derived from a named ticket (`leakage_audit = leaking`; the amendment
cap; `parameter_fragility_map = knife-edge`; `regime_failure_search = current is worst`). The
model still cannot create one. `diagnose` assembles the `DiagnosticBundle` and detects
contradictions with memory (AGENT-007 §2.2.1) at this stage.

**Files:** `crates/ledger/src/negative.rs`, `crates/harness/src/lifecycle/diagnose.rs`.
**Acceptance:** each new class is produced by its ticket and by nothing else; a fixture with a
knife-edge fragility map yields `fragile_parameters`; the agent tool cannot create any class.
**Traces:** AGENT-013 §5.8, §8.2; AGENT-007 §3.2 (amended).

### S-10.8 A `supported` verdict needs complete evidence and a reproducibility receipt — M
`PARTIAL` 2026-09-17 — **the battery and the receipt are built; the nightly job is
not.**

`evaluate` already refuses `supported` without all three tickets *and* a receipt
(S-10.4). `crates/backtest/src/receipt.rs` is the receipt itself: every input that
can change a number — data by content, feature registry, calendar, engine, strategy
by content, costs, platform seeds, harness. Every field required, no serde default;
a receipt missing the calendar version is not a receipt with a gap, it is one that
cannot detect the one class of drift this platform has actually seen.

**There is no tolerance in `reproduce`, deliberately.** It asks whether the platform
computed the same thing, not whether it computed something close enough — and a
tolerance would hide exactly the small systematic drift a calendar or cost-model
change produces.

The digest separates its fields. Without the separator `("ab", "c")` and
`("a", "bc")` hash identically, and two receipts that differ would report as the
same — the one failure a receipt must not have.

Drift under an **unchanged** receipt files a `platform_defect`; a changed receipt
does not, because filing one for a deliberate change trains everybody to ignore the
defects. `drift_rate` over an empty sample is `None`, not zero: "0% drift over
nothing sampled" reads as a healthy platform.

A test corrected a claim I had written wrongly: a result that was NaN and stayed NaN
is **bit-identical**, not drift. The platform computed the same thing both times; it
has been broken all along, which is a different problem and one this detector would
otherwise file every night.

**AT-63 caught a real leak.** The receipt's first version carried the platform seeds
as values, and a receipt travels to the agent — so a type holding them hands them
over. It carries a *digest* now, which detects the drift a receipt exists to detect
and reveals nothing that would let a caller reproduce a favourable draw.

**Not done:** `lifecycle.reproduce`, the `repro_drift` nightly job over a 2% sample
of 30 days, and the automatic `platform_defect` filing.

`evaluate` requires out-of-sample, cost-model and benchmark tickets *and* a `ReproSpec` on
the experiment before it may write `supported`; missing any → `inconclusive: evidence_incomplete`.
The receipt: dataset hash, feature registry version, calendar version, engine version, strategy
version hash, cost model version, platform seeds, harness version. `lifecycle.reproduce`
re-runs under the receipt and reports bit-equality (no trial: a re-run of a registered look is
not a new look, ADR-P2-17). A nightly job reproduces a 2% sample of 30 days and publishes
`repro_drift`; non-zero drift files a `platform_defect` automatically.

**Files:** `crates/backtest/src/receipt.rs` (new), `crates/mcp-server/src/tools/lifecycle.rs`,
`crates/jobs/src/kinds/repro_drift.rs`, migration 0090.
**Acceptance:** removing any one of the three tickets or the receipt from a fixture renders
`inconclusive: evidence_incomplete`; `reproduce` reports bit-identical on a recorded
experiment and a diff after a deliberate calendar-version bump; the nightly job files the
defect on drift.
**Traces:** AGENT-013 §6.9, §8.4; §13.8, §13.9. SPEC §2, §3.1.

## Rings 1 and 2

### S-10.9 Proposals: the only doors into rings 1 and 2; the kill switch reaches ring 0 — M
`PARTIAL` 2026-09-17 — **the door is built; the removal of the other write paths is
not.**

`crates/ledger/src/proposal.rs`, `lifecycle.propose`, `POST /api/lifecycle/proposals`
and `0128`'s `agent_lifecycle.proposal`. The row is **inert**: an agent creates it, a
human decides on the desk, and the registry write happens at that moment as that
human. The response says `"effect": "nothing yet"` out loud, because a model that
thinks it promoted something will report that it did.

**Creating one is not gated, and that is deliberate.** The envelope is on the
*decision*, because that is where something happens; gating the asking would teach a
run that asking is expensive, and a run that has learned that looks for another way
to get what it wants.

Nothing is proposed while the kill switch is tripped — for every kind, not just ring
2 — because a tripped switch is the platform saying something is wrong *now*, and a
proposal created during it sits in the queue looking reasonable when somebody resets
the switch an hour later. The switch is read from `monitor.kill_switch`, and **a read
that fails answers "tripped"**: an outage is not evidence that the platform is
healthy.

Ring 2 additionally needs the battery and the readiness check. Ring 1 does not — the
registry is where a strategy lives, not where it trades, and a candidate the battery
rejected is still worth writing down.

`a_decision_names_a_human_and_a_time` makes "who approved this" a fact rather than a
reconstruction from timestamps. `Decision::permits_write` returns true only on an
explicit approval: "not yet rejected" is how an approval queue becomes a delay.

**Not done:** the desk surface, the approval route that performs the write as the
human, and — the large one — **removing every other write path from an agent token
into the promoted set**, with the static test that proves it. The ring-2 door is
already closed and tested (`the_promotion_envelope_is_the_only_door_to_paper`); the
ring-1 paths are not, and removing them is a product change rather than a
refactor.

`Proposal` (AGENT-013 §7.5) with `kind`, `ring`, `dossier_ref`, `envelope`. `lifecycle.propose`
creates it; `promote_strategy` and `change_limits` carry `Promotion`, `deploy_paper` carries
`Spend`. Creation checks `precapital_battery ≠ reject` and `go_live_readiness ≠ no_go` for
ring 2 and **refuses while the kill switch is tripped**. Approval is a human action on the desk
and a `decision` row; the registry write happens then, as that human. **Every other write path
from an agent token into the promoted set is removed**, and a static test proves no tool with
`Risk::Write` targets a registry table outside the proposal path.

**Files:** `crates/ledger/src/proposal.rs` (new), `crates/mcp-server/src/tools/lifecycle.rs`,
`crates/api/src/routes/proposals.rs`, `crates/invariants/tests/static_invariants.rs` (new
check), migration 0091.
**Acceptance:** no agent tool writes the promoted set (static); `propose{promote_strategy}` is
`Promotion`-enveloped and changes nothing until approved; `propose{deploy_paper}` is refused
while tripped and succeeds after reset; the approval is a `decision` row with the human as actor.
**Traces:** AGENT-013 P6, §4, §6.8, §7.5, §8.6; §13.10, §13.11. COMP-007 §5.

## Comparison, optimisation, memory, yield

### S-10.10 `compare_candidates` — SPEC §11.5 as one door — M
`PARTIAL` 2026-09-17 — **the door is built; the tool that walks through it is not.**

Everything §11.5 asks for already existed in `backtest::stats::compare` — paired
replicates, anytime-valid confidence sequences, the ROPE, the win-rate rule, Holm,
the Multiple Comparison Matrix, `ComparisonPlan::judge` — and **nothing outside that
file called any of it**. The tenth instance of this repository's dominant defect
shape. What shipped was `compare_backtests`, which puts headline metrics side by
side and lets a reader form an impression.

`crates/research/src/compare.rs` is the sweep: every pair judged, Holm across them,
the matrix assembled. It re-implements none of the statistics, because a second
implementation is a second standard and the looser one is the one a candidate gets
judged by.

**Only `Promote` establishes an edge.** `Reject` means A's advantage is smaller than
the declared delta — §11.5 says in as many words that it is *not* "B is better" — so
the reverse direction is judged rather than inferred, and a sub-delta advantage
ranks nobody.

The ranking is a win **count**, not a tournament resolution: `A ≻ B` and `B ≻ C`
does not give `A ≻ C` under a paired comparison, and inventing the transitive edge
would state evidence nobody produced.

A test found a real property worth knowing: **the confidence sequence's width
depends on the declared ROPE scale, not on the observed spread.** With a half-width
of 2.0 and a difference of 1.0, twenty-nine replicates leave the lower bound just
below zero and the protocol says `KeepSampling` — correctly. Declaring a loose bound
costs replicates, and a caller should know that before declaring one.

There is no seed anywhere in the module, and a test greps for one. The sweep's
output type is named `Sweep` rather than `Comparison`: a static invariant greps for
`Comparison {` constructed outside the protocol, and a second type of that name
defeats a name-based guard. The invariant caught it.

**Not done:** the `compare_candidates` tool and its route, which need measurements
from real backtest runs; and the desk surface. AT-62 still stands — no
critical-difference diagram exists anywhere.

All nuisance sources randomised with **platform-held seeds** (the tool has no seed argument —
ADR-P2-02's technique), ~29 paired replicates, `A ≻ B` iff `P(A>B) ≥ 0.75 ∧ P(A>B) − CI_lower > 0.5`,
Holm across pairs, anytime-valid confidence sequences with ROPE ±`delta_practical`. Output is a
Multiple Comparison Matrix; the desk renders no critical-difference diagram (AT-62 stands). The
`compare` workflow cannot rank without it. Calls `ComparisonPlan::judge` where it exists
(HANDOFF §2) rather than re-implementing it.

**Files:** `crates/research/src/compare.rs`, `crates/mcp-server/src/tools/lifecycle.rs`.
**Acceptance:** two copies of the same strategy → `indistinguishable`; a planted dominant
candidate → ranked first with the stated criteria; the seed field does not exist in the schema
(static); Holm is applied (asserted on a three-way fixture).
**Traces:** AGENT-013 §3 (`compare`), §8.3; §13.12. SPEC §11.5.

### S-10.11 `optimise`: the `ResearchRun` child worker is a worker sub-run — L
`PARTIAL` 2026-09-17 — **the worker, the door, the projection and the decision are
built; the parent does not resume, because `Effect::Await` resumption does not
exist in this driver.**

**What was actually broken.** `campaign_driver::child_kind` has mapped DIAGNOSE to
`JobKind::ResearchRun` since it was written, and **nothing anywhere registered a
worker for that kind**. A campaign reaching its second phase submitted a child job
no pool would ever claim, and `await_child` polled it every five seconds forever.
The driver was correct, the phase machine was correct, the idempotency was correct,
and no campaign could get past DIAGNOSE. HANDOFF §2 lists this worker as missing and
it was missing in the strongest sense: the caller existed and the callee did not.

HYPOTHESIZE was worse and quieter. It had *no* child kind at all — described in the
source as "the agent (or a human) proposing" — so the driver wrote the event and
moved straight to EXPERIMENT, which ran whatever the previous cycle had left in
place. A campaign's search never changed what it was searching.

**Built.**

* **`research::propose`** — `Move` as the eight typed variants of FEAT-003 §8.3,
  `Proposal { move, hypothesis, ranges }`, and a grammar emitted from beside the
  type. Not a string: the one place a model changes the *structure* of the search
  was prose, and prose is the bug class this repository keeps paying for. Each move
  names the check it still owes through `Pending`, so "validated" here never claims
  the graph validator, the feature lane, the regime labels or the simulator's
  capability list ran — four of the eight need machinery that is elsewhere.
* **`harness::subrun`** — `Role::{Critic, Worker, Reader}` and `Delegation::check`,
  which refuses a role whose projection does not match it. A critic dispatched under
  `full` would run, return a verdict, and be recorded as a critic; this is the one
  place that cannot happen.
* **`context::projection::campaign_worker` + `brief`** — the pinned set is *derived*
  from the parent's store by kind, so the parent's reasoning is excluded because it
  is not there. `adding_reasoning_to_the_parent_never_widens_the_worker_brief` is
  S-9.20's test pointed at the other projection, and it matters more here: a critic
  that was led gives a soft verdict, while a worker that was led returns the
  parent's own move wearing a second opinion's name.
* **`ResearchRunWorker`**, registered in the research pool. HYPOTHESIZE reads the
  pinned brief, calls under `Proposal::schema()` at temperature zero, validates, and
  retries exactly once with the validator's reason. DIAGNOSE is handled too —
  because a campaign cannot reach HYPOTHESIZE without it — and is honest about being
  bounded: it attaches the last phase's job result under `bundle_kind:
  "phase_result"`, which is **not** FEAT-003 §8.2's `DiagnosticBundle`.
* **`define_campaign` / `get_campaign`** under `research`, and
  `POST/GET /api/lifecycle/campaigns`. The six DEFINE facts a registration already
  fixes are **absent from the schema and refused by name** if sent, so §8.5's "one
  object" is a mechanism rather than an instruction. The exploration floor is
  neither offered nor accepted (INV-21), asserted both ways.
* **The driver** claims `(campaign, phase, seq)` before dispatching — the opposite
  order from its child jobs, because here the row is what carries the job id — and
  logs the `propose` decision with the move vocabulary as its candidate set.

**Not done, and why. The run does not suspend, and it does not resume.** §8.5 says
it "emits `Effect::Await { job_id }`" and it does not, deliberately:
`Input::JobAccepted` — the input that produces an `Await` — **has no producer in
either driver**, so `Effect::Await` has never been reached by a real run, and the
one thing `local_driver` does with an `Await` is end the run as
`Fenced(BackendFailure)` with the message "this driver cannot resume a suspension
yet". Wiring `define_campaign` to suspend today would therefore kill a healthy run
and label the death a backend failure. Instead the tool returns the campaign id and
the driver job id as an ordinary result, the campaign runs to its terminal state in
the job service, and `get_campaign` reads it. Correcting this is one change in the
driver and one in the `Await` arm, and it is what this task waits on.

Also not built: the `DiagnosticBundle` proper, and the `BudgetLease` split AGENT-010
§9 describes — the worker runs on the parent's profile and the campaign's own budget
bounds it, which is a weaker guarantee and is stated here rather than implied.

**Decisions:** ADR-S10-03 (two hashes, not one), ADR-S10-05 (the brief is pinned at
DEFINE), ADR-S10-06 (propensity 1.0, and the worker refuses a profile that would
make it unknowable).

**Depends on:** S-2.12 (sub-runs), S-9.20 (projections), Phase 8 workflows.
**Files:** `crates/api/src/campaign_driver.rs`, `crates/api/src/agent/subrun.rs`,
`crates/mcp-server/src/tools/campaign.rs` (`define_campaign`, `get_campaign` under `research`).
**Acceptance:** a campaign advances through HYPOTHESIZE via the worker sub-run on the reference
fixture; the worker's context provably excludes parent reasoning (S-9.20's test, reused);
AT-59 still passes with the agent in the loop; the profile that hides `optimise` has a
conformance reader.
**Traces:** AGENT-013 §3 (`optimise`), §8.5, §11; §13.13, §13.15. PHASE-2-5-PLAN §2.1, ADR-P2-04.

### S-10.12 Open questions are memory — M
`PARTIAL` 2026-09-17 — **the kind, the closing rule and the run-end write are built;
the deliberate tool and the browser tab are not.**

A run notices something it cannot answer, and that noticing used to die with the
conversation: the next run rediscovered it, spent a step on it, and noticed it
again. `harness/src/memory/questions.rs` makes a question an entry like any other —
subject, retrieval, closure — and `Kind::OpenQuestion` and the brief's 10% slot
already existed.

**The model cannot close one.** Same discipline as the verdict and for the same
reason: a run that has been thinking about something for nine steps genuinely feels
it has resolved it. The platform closes a question when a later finding *cites* it,
which is a fact about what was written down. The first answer is the one recorded —
a second close would overwrite who answered it.

Citing a question the store does not hold closes nothing and is **not** an error. A
run that has learned citing is risky stops citing.

`blocking` leads the brief slot whatever a question's stated value, because a brief
that buried it under six curiosities would let the run repeat the mistake the
question exists to prevent. `value` is the asker's estimate and scores nothing —
`Yield` already pays zero for an opened question, and a value field that fed a
metric would be a run grading its own output.

The loop writes `WorkingState.open_questions` at run end **for every outcome**: a
fenced run's questions are not less true, and the run that hit the thing it could
not answer is often the one that failed. Those entries say `why_it_matters: not
stated` rather than inventing stakes — which is exactly what
`lifecycle.open_question` exists to improve on.

**`open_question` landed 2026-09-17**, with `POST /api/lifecycle/questions`.
`why_it_matters` is required and has no serde default, which is the whole
difference between this door and the run-end write: the loop records what a
reflection noticed and says the stakes were never stated, because prose does not
contain them however confidently it is read.

**There is no field for `status`, and sending one is refused by name rather than
ignored.** §9's rule, and ignoring it would let a run believe it had closed a
question and report accordingly. The schema offers no closing field either, so the
refusal is a second line rather than the only one.

**One serialisation, two writers.** `OpenQuestion::extra()` is now the single
producer of the five `extra` keys, and `questions::noticed` the single home of the
"stakes not stated" sentinel — before this each path built its own object, which is
the arrangement where one grows a key and the other's rows silently lack it.
`both_writers_produce_the_same_extra_keys` is what keeps them one shape, and
`stakes_were_stated` is what keeps them distinguishable, which is why they are two
paths at all.

**No migration was needed.** The plan names one extending `0055`; `agent_memory.entry`
already holds the kind and `extra` already holds the kind-specific fields — ADR-S4-01's
argument, and `questions` is already a readable bucket with a test asserting every
kind is reachable from one.

**Not done:** `OpenQuestions.tsx`, and the platform-side closing sweep (a later
finding's citation closes a question — `close_cited` is built and has no caller
outside its tests).

New semantic kind `open_question` (AGENT-007 §2.2 amended) with `why_it_matters`, `blocking`,
`value`, `status: open|closed{by, how}`; written by `lifecycle.open_question` and by the harness
from `WorkingState.open_questions` at run end; closed by the platform when a later run's finding
cites it. Brief slot (AGENT-007 §4): up to 10% of `Reference`, `blocking` first. Memory browser
tab (AGENT-012 §7).

**Files:** `crates/harness/src/memory/questions.rs`, migration 0091 (extends 0055),
`frontend/src/components/desk/memory/OpenQuestions.tsx`.
**Acceptance:** a question opened in one conversation appears in the next run's brief for the
same subject; a finding citing it closes it with `how = answered`; the tab lists and filters
them; retrieval logs `cited`.
**Traces:** AGENT-013 §9; §13.14. AGENT-007 §2.2, §4 (amended), §8.9.

### S-10.13 Yield — what the run produced, as a number — M
`DONE` 2026-09-17 — `harness/src/lifecycle/yield.rs`, migration `0128`, computed in
`local_driver` as the run goes rather than reconstructed at the end: the end is
where a fenced or cancelled run has *least* state, and a yield only computable on
the happy path would report every failure as having produced nothing.

**The weighting is the design**, because a yield metric is a thing runs get
optimised against the moment it exists. A suggestive finding is worth nothing — it
is a lead, and counting it would make "record six weak findings" the cheapest way to
look productive. An opened question is worth nothing; a run that opened four and
answered none left more work than it did. A proposal is worth nothing; the decision
is where the value is. A negative whose class is new for the subject is worth two,
because it closes a door nobody had closed — and a yield metric that paid only for
positives would be one that quietly stopped reporting failures.

A **refuted** finding counts. Knowing an idea is dead is the point of keeping it,
which is the argument `Confidence::weight` already makes about retrieval.

`knowledge_units` is deliberately *not* stored: it is arithmetic over the counts,
and a stored copy would drift the first time the weighting changed — which it will.
`barren` *is* stored, because the scorecard groups by it, and
`a_barren_run_spent_a_trial` keeps it honest.

**Not done:** the scorecard's barren column and the usage page. `barren_rates`
exists and is tested; nothing reads it yet.

The platform computes `Yield` (AGENT-013 §10) at run end from the effect log and memory
writes: findings by confidence, negatives and whether the class is new for the subject,
questions opened/closed, contradictions resolved, proposals, hypotheses tested/refuted/supported,
cost, `cost_per_knowledge_unit`. A run with trials and zero knowledge units is **barren** and
reported per tier and per workflow in the scorecard (AGENT-009 §4 amended) and on the usage page.

**Files:** `crates/harness/src/lifecycle/yield.rs`, `agent_runs` columns (0091), `evals/scorecard`.
**Acceptance:** every finished run has a `Yield` row; a barren fixture is flagged; the scorecard
carries barren rate per (tier, workflow); the usage page shows cost per knowledge unit.
**Traces:** AGENT-013 P5, §10; §13.14. AGENT-009 §4 (amended).

### S-10.14 Workflows as stage skeletons, per tier — M
`PARTIAL` 2026-09-17 — **the skeletons, both enforcement halves and per-tier
availability are built; the authoring mode and the workflow-on-every-trial are
not.**

**Per-tier availability landed with S-10.11**, which needed it: `Profile::workflows`
is a declared list with `Profile::may_run` as its enforcement point, the three
shipped profiles declare §11's three rows, and `local_driver` refuses a run whose
workflow the tier does not offer — a third admission refusal beside the two that
were already stated before anything is spent. It **refuses rather than downgrades**:
a campaign quietly demoted to an `investigate` would still finish, still write a
verdict, and still read as the answer to the question that was asked.
`validate` closes the other direction — a profile below `local_high` cannot
*declare* `optimise` — and `conformance.rs` has two readers, one over the shipped
configuration and one over a mutated copy. ADR-S10-04 records why the default is
the smallest set rather than all seven.

`harness/src/lifecycle/workflows.rs` holds all seven as a table: required stages,
skippable stages, verdict vocabulary, and whether the workflow may spend a trial at
all. `may_transition` refuses a transition that skips a required stage and names the
stage it skipped.

**`survey` cannot spend a trial, in two places.** Exposure hides every
trial-spending tool from a survey run, which costs nothing because the model never
reaches for one; `CallGate::spending` refuses at the door, which is what holds when
a later code path routes around the filter. Two reasons for a spending ban now share
one field — `Spending::{Allowed, ChatTurn, Survey}` — because two booleans is the
shape where a third reason arrives and the refusal site grows a condition nobody
tested. The codes differ: "start a research run" is right for a chat turn and wrong
for a survey, which is one already.

`falsify` skips `hypothesise` and cannot skip pre-registration, asserted directly —
§3 calls that one out by name, and an attack protocol chosen after seeing which
attack works is p-hacking with extra steps.

The §3 step names and this crate's `Stage` enum are **not the same list**, and the
mapping is written into the module header rather than left for a reader to infer
something was forgotten.

**The authoring mode landed with S-10.3** —
`free | skeleton_grammar | skeleton_confirm`, validated against the tier (`free` is
refused below `local_high`), with two conformance readers and the skeleton it
selects actually built. A mode that named a behaviour nothing performed would have
been the field-as-comment this file exists to prevent.

**Not done:** the workflow on every *trial* (it is on the run row; the ticket
carries the pre-registration hash but not yet the workflow).

The seven skeletons (AGENT-013 §3) as a table in `crates/harness/src/lifecycle/workflows.rs`:
required stages, skippable stages, verdict enum. The loop refuses a stage transition the
skeleton does not permit and never skips a gate. Per-tier availability and the pre-registration
authoring mode (`free|skeleton_grammar|skeleton_confirm`) and refinement caps (AGENT-013 §11)
are profile values with conformance readers. Workflow is recorded on every trial the run spends.

**Files:** `crates/harness/src/lifecycle/workflows.rs`, `config/profiles/*.yaml`,
`crates/harness/tests/conformance.rs`.
**Acceptance:** `survey` cannot obtain a ticket; `falsify` cannot skip pre-registration; on
`qwen2.5-7b` `investigate` completes with a grammar-filled registration and ≤ 1 refinement;
the ledger answers "how many trials were replications" from the workflow column.
**Traces:** AGENT-013 §3, §11; §13.15.

## Desk and measurement

### S-10.15 The desk shows the stage, the registration, the falsifier, the proposals, the yield — L
`NOT STARTED`

Composer mode control → *Chat / Research* with workflow chips under Research, the Continuous
toggle (its charter form is S-11.13) and the SLv2 editor as an action (AGENT-012 §3.3 amended). New cards: **Stage** (the ten-stage
strip with the current one lit and what would move it), **Pre-registration** (readable before
the run spends; *steer* opens an amendment with the lineage warning), **Research offer**
(S-10.2). Hypothesis cards show the falsifier and the platform's verdict with the numbers that
decided it. **Proposals** appear in *Needs you* with their envelope and the tickets that
licensed them; the ring is in the conversation header. Yield in the Report card and on
`/desk/usage`. The ten stages are keyboard-navigable in the replay scrubber.

**Depends on:** Phase 6.
**Files:** `frontend/src/components/desk/cards/{Stage,Preregistration,ResearchOffer,Proposal}.tsx`,
`Composer.tsx`, `DeskHome.tsx`, `UsagePage.tsx`.
**Acceptance:** a running Research turn shows mode, workflow, ring and stage in the header and
State tab; the pre-registration card renders before any trial and its steer creates an
amendment; a proposal card shows envelope and tickets and its approve/deny work from the
keyboard; both themes, both densities, 375 px.
**Traces:** AGENT-013 §12 phase 6; AGENT-012 §2, §3.2, §3.3 (amended), §19.9.

### S-10.16 The `judgement` suite — research decisions as a scored thing — L
`PARTIAL` 2026-09-17 — **the scoring is built; the thirty cases per tier are not.**

`harness/src/eval/judgement.rs` scores a run against **planted ground truth** —
which is what the other four suites cannot do, because there is nothing to compare
an answer against: a run that reports "no edge" and one that reports "an edge" both
look like a run that reported something.

Six criteria, and the selection principle for them is that **each is a way of being
wrong that looks like working**: reporting a planted null as supported, refining
past the point the chain stopped paying (looks like persistence), choosing the
broader workflow (looks like thoroughness), sealing a falsifier nothing can evaluate
(looks exactly like pre-registration), re-testing an idea memory records as dead
(looks like diligence), and ending with trials spent and nothing written.

**Never one number.** `Score::passed` requires every applicable criterion: a run
that got five of six right and reported a planted null as supported has not scored
83%, it has produced a confidently wrong answer about whether money should be
risked. The same argument AGENT-009 §4 makes about averaging across tiers, applied
within a case.

A criterion the case cannot exercise is `not_applicable` and **not a pass** —
counting one would let a narrow case score like a broad one, and the suite would
drift toward narrow cases.

Missing a planted leak fails even when the verdict was right: the right answer for
the wrong reason is what this suite exists to tell apart.

**Not done:** `evals/judgement/**` — the ≥ 30 cases per tier with their tapes —
the scorecard row, the sealed 20% holdout over them, and `evals/runner/run.py`. The
cases need recorded trajectories (gate H-2); the scoring does not, and is what will
read them.

AGENT-009 §3 gains `judgement`: cases with **planted ground truth** — synthetic datasets with a
known edge or a known null, a known-leaking feature, a pair of candidates with a known dominance,
a dead idea already in memory — scoring whether the run rejected when it should, stopped
refining at diminishing returns, chose the workflow the question shape called for, sealed a
registration whose falsifier could be evaluated, avoided the duplicate at intake, and produced
non-zero yield. Per tier, per workflow; replay where the tape is fixed, live nightly otherwise;
gated like every other suite (AGENT-009 §4). This is what makes "quality of research decisions"
a scorecard row, and what AGENT-014 §8 reads before a frontier profile may run continuous research or go `autonomous`.

**Depends on:** S-4.8, S-4.9, S-4.11.
**Files:** `evals/judgement/**`, `crates/harness/src/eval/judgement.rs`, `evals/runner/run.py`.
**Acceptance:** ≥ 30 cases per tier; a harness that reports a planted null `supported` fails the
suite; a run that re-tests the planted dead idea fails; the scorecard shows `judgement` per
(tier, workflow); the sealed 20% holdout applies (S-4.17).
**Traces:** AGENT-013 §10, §11; AGENT-009 §3, §4, §8.8 (amended); AGENT-014 §8.

---

## Exit criteria

- No trial is spent by any run without a sealed pre-registration, and none by a Chat turn at all.
- Every hypothesis has a falsifier the platform can evaluate, and the platform — never the model —
  writes the verdict.
- A refinement cites a diagnosis and predicts its effect; chains stop at diminishing returns.
- A `supported` verdict has out-of-sample, costs, benchmark and a reproducibility receipt behind
  it, or it is not `supported`.
- The only door from an agent into rings 1 and 2 is a proposal a human decides; ring 3 has no door.
- Every run reports its yield; barren runs are visible per tier and per workflow.
- `judgement` is a scorecard row.
