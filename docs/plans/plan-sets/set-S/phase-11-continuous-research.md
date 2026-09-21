# Phase 11 — Continuous research: Research with the toggle on

**Completion: 0% (0 / 14 tasks), plus 6 `PARTIAL`** — S-11.1 (the object, the fold, the driver), S-11.3 (the scored agenda) and S-11.14 (the API and the three read tools) landed 2026-09-21; each names what it does not cover.

> **The four pure modules have a caller, 2026-09-21 — and wiring them found a
> live bug in the driver that every test had passed over.**
>
> `iterate` tested `ended == "succeeded"`, and `agent_runs.status` **never takes
> that value**: the run drivers write `completed | failed | fenced | refused |
> suspended | cancelled | offered`. So **every child this driver has ever
> finished took the failure branch**, re-queued, and expired at two attempts. No
> test caught it because none exercised the driver against a real run status.
> `verdict_of` is now the one mapping between that vocabulary and
> `harvest::FAILURE_VERDICTS`.
>
> `schedule` gates the loop **before** the agenda is read — an item scored outside
> a window is work the driver was not allowed to plan. `harvest` offers before it
> settles. `stall` gets its `FinishedRun`s from a five-way join, because
> `Folded::finished` is only `item → verdict`. `digest` counts its cadence **over
> the period, not over the session**: `Folded::runs_started` is a session total
> and cannot answer "three runs, two since the last digest".
>
> **api lib: 329 → 351 tests**, and each new one proves *the call happens* rather
> than that the module works — the modules' own 108 tests already prove the second.
>
> **The trials-versus-dollars trap was made loud rather than quiet.** `0139` adds
> `session_digests.spend_unit`, **NOT NULL with no default**, CHECKed to
> `('trials','usd')`, so no writer can leave the question open; the unit is on the
> `digest_written` event too. It is **not finished**: the digest's own sentence
> still reads `"{:.2} spent over 2 runs"` with no unit in it, and that line is in
> `research-session`, outside the track's territory. Until the route selects
> `spend_unit` and the card renders it, a reader sees trials where they expect
> dollars.
>
> Two spec problems, neither implemented. **§4.8's "marked worth pursuing" is a
> mark nothing records** — `next_moves` is not a field of `mlops.dossier.body`
> and `worth_pursuing` appears in no table, so source 4.8 is wired and **inert**;
> unmarked hypotheses land in `Harvest::skipped` and reach the log with the
> reason. And **§7.1's `proceed_as_variant` is a vocabulary nothing writes** —
> `hypothesis_precheck` files `blocked | duplicate | new`, so read literally the
> circling rule can never fire. `duplicate` is read as the variant (§7.1's own
> gloss is *"the dedupe did its job and the policy overrode it"*), and the count
> of runs with **no** precheck verdict rides on `session_stalled.not_measured` so
> the rule's reach is visible rather than assumed.

**Requirement IDs:** PG-01…PG-14.
**Spec:** [AGENT-014](../../../specs/AGENT-014-continuous-research.md);
[ADR-0037](../../../adr/0037-two-modes-continuous-toggle.md).
**Depends on:** Phase 10 (lifecycle, yield, proposals, open questions, `judgement`), Phase 2
(effect log, leases, resume), Phase 4 (memory kinds, the brief, profiler), Phase 7 (monitor
events, kill switch). S-11.13 depends on Phase 6.
**Blocks:** nothing.
**Migration range:** 0092–0096.
**Parallel with:** Phase 9.

The user sees two modes, Chat and Research, and under Research a **Continuous** toggle. With
it on, the question becomes a charter and the run becomes an agenda of runs. Internally that is
one object, `ResearchSession { continuous }` — a bounded Research run is the one-item case —
and the continuous machinery is a fold over an append-only event log driven by a job
(ADR-P2-04's pattern, reused because it already survives a killed driver, AT-59) whose children
are Research runs, each holding a budget lease. "Endless" is the session's `replenish`
setting. Fourteen tasks: the object and its fold; the nine agenda sources; scoring with a
forced exploration share and a `decision` row per choice; three-layer dedupe; execution with
leases and harvest; schedule windows; the couplings to the kill switch, the tier and the
platform-defect breaker; the stall detector; autonomy and the frontier-only rule; observation
subscriptions; replenish; the digest; the desk; the tools and API.

**What this is not.** Not a third mode (user decision, 2026-09-15); not `mlops.campaign`
widened (a campaign optimises one family under one DEFINE; a session decides which family);
and not a session that approves anything (every ring-1/2 proposal waits for a human in every
autonomy setting).

---

## The object and its loop

### S-11.1 `ResearchSession`, `Charter`, `AgendaItem`, the event fold and the driver — L
`PARTIAL`

Tables `research_sessions` (with `continuous`, `question`, `charter`, budget, schedule,
autonomy, `replenish`, `review_every`), `session_events (session_id, seq, kind, payload_json,
created_at)` with a unique `(session_id, seq)` and a BEFORE INSERT transition trigger (the 0052
discipline), `agenda_items`, `session_digests`. **Every Research conversation gets a session
row**: a bounded run is `continuous = false` with one agenda item, so the shape is one.
`Charter` (AGENT-014 §2.2) with `exploration_share` whose constructor refuses < 0.10 and whose
column CHECKs it; `standing_constraints` written as `user_preference` rows scoped
`session:<id>`. `JobKind::ContinuousResearch`: leased, heart-beaten, re-queued; one iteration
per AGENT-014 §3; children through `JobStore::submit_child_once` keyed
`(session_id, agenda_item_id)`. Phase 1 scope: sources 4.1/4.2, `supervised`, `replenish = false`.

**Files:** `crates/research-session/` (new crate: `fold.rs`, `charter.rs`, `agenda.rs`),
`crates/api/src/session_driver.rs`, `crates/jobs` (`JobKind::ContinuousResearch`),
`crates/api/src/agent/manager.rs` (bounded runs create the one-item session), migration 0092.
**Acceptance:** killing the driver between claim and `run_started`, then restarting, yields one
child, one lease, an identical fold (AT-59's form); an illegal transition is refused by the
trigger; `exploration_share = 0.05` is refused by the constructor and by the CHECK
(adversarial, both halves); a bounded Research run has a session row with one item.
**Traces:** AGENT-014 P1, §2, §3, §10; §12.1, §12.3 (floor half), §12.12 (bounded half).

**Built 2026-09-21. `PARTIAL`, and the two halves are named rather than blurred.**

**What runs.** A new crate `crates/research-session` holds the parts that are
decisions rather than plumbing, so they test without a database: `charter.rs`
(the charter, the budget, the schedule, `Autonomy`, and `ExplorationShare` as a
*sealed type* whose only constructor refuses below 0.10 — including through
`serde`, because a charter posted as JSON is a way in), `agenda.rs` (`Source`,
`ItemStatus`, `Score`, the dedupe key and layer one of the three-layer refusal)
and `fold.rs` (`EventKind`, `SessionState`, `transition_ok`, `fold`). 26 unit
tests. `migrations/0138_research_sessions.sql` carries the four tables, the
BEFORE INSERT trigger that assigns `seq` and refuses an illegal transition, and
the partial unique index that is layer one. `api::sessions` is the store,
`api::session_driver::SessionWorker` is the driver, `JobKind::ContinuousResearch`
is its kind, and it is **registered** in `apps/platform/src/main.rs` — a driver
nothing claims is the defect S-10.11 was.

**Verified against a live Postgres (5433), not merely written.**
`crates/invariants/tests/a_session_log_refuses_what_the_fold_would.rs` — seven
cases, all green: `exploration_share = 0.05` is refused *by the column* and so is
a charter with the key missing (`NULL >= 0.10` is NULL, and a CHECK that
evaluates to NULL passes, which no text scan can catch); the log opens in `draft`
and nothing else; `running -> draft` is refused; nothing appends after `stopped`;
`seq` comes out gapless with no writer choosing it; two live items cannot share a
dedupe key but a *finished* question can be asked again; an item cannot climb
past two attempts; a conversation gets at most one session; and neither the log
nor a digest can be rewritten. Two static suites hold the two copies of each rule
equal — `a_session_transition_table_has_one_definition` walks all 36 ordered
pairs of state words and compares Rust to SQL cell by cell, and
`exploration_floor_is_one_number` reads the floor out of Rust and looks for it in
the migration.

**What is not built, and is not pretended to be.** The **lease** half of
acceptance 1 (S-11.5 owns leases; nothing here bounds a child's spend), scoring
(S-11.3 — selection is oldest-claimable with `user` first, which is §5.4's
tie-break on an unscored agenda, and the `selected` event says so in words rather
than writing an empty candidate set a later reader would take for "it considered
nothing"), the harvest, the stall detector, windows, the digest, and sources
4.3-4.9. Acceptance 4 — a bounded run has a session row with one item — is
**wired** through `AgentManager::ensure_session` and covered at the store level,
but has not been exercised end-to-end through a real turn.

**One judgement call, recorded rather than assumed: ADR-P11-01.** AGENT-014 §3
names `JobStore::submit_child_once` and §10 says children go "via the
conversation/turn path". Those are two mechanisms and a child is created once — a
conversation turn is not a job. The build takes §10's path and gets §3's stated
*property* from a derived conversation id (`uuid5(session_id ++ item_id)`, so
there is no second id a second conversation could open under) plus a conditional
UPDATE out of a claimable status (so exactly one of two drivers wins). The spec
wording is written up as **Q-S11-01** in `OPEN-QUESTIONS.md` and left open; the
build is not blocked on it.

**One small contract change:** `AgentManager::send_message` now takes
`Option<Workflow>`. The front door passes `None` (no chip yet, AGENT-012 §3.3);
the session driver passes the agenda item's, so a run started to replicate a
stale finding says `replicate` on its row *and* in its loop instead of claiming
to be an investigation. One value feeds both. `agent_conversations` also gained
`parent_conversation_id`, which groups the rail (§9) and is how the front door
tells a child of a session apart from a conversation a human opened.

### S-11.2 Memory-fed sources: open questions, retry conditions, contradictions, stale findings, follow-ups — M
`NOT STARTED`

Sources 4.3–4.6 and 4.8 as subscriptions the driver evaluates each iteration over memory and
the ledger: in-scope `open_question`s (`blocking` first), `negative_result`s whose
`retry_condition` now evaluates true, unresolved contradiction pairs (→ `diagnose`),
`established` findings past `valid_until` (→ `replicate`), and finished runs' proposed next
questions. Each yields an `AgendaItem` with `source`, `source_ref`, `dedupe_key`.

**Depends on:** S-10.12, S-4.4, S-4.15, S-4.16.
**Files:** `crates/research-session/src/sources/{memory,ledger}.rs`.
**Acceptance:** a retry condition flipping true (fixture adapter appears) inserts an item within
one iteration; a contradiction pair inserts a `diagnose` item; a stale finding inserts a
`replicate` item; each carries its `source_ref` and opens in the desk.
**Traces:** AGENT-014 §4.3–4.6, §4.8; §12.8 (memory half).

### S-11.3 Scoring, exploration, and the decision row — M
`PARTIAL`

`total = value × novelty × feasibility / cost` over percentile-normalised signals
(AGENT-014 §5.1); the weights are a versioned `SelectionPolicy` with a conformance reader per
field; `feasibility` is 0 for a workflow outside `permitted_workflows`. `user` items bypass
scoring. With probability `exploration_share`, uniform over feasible novel items outside the
top-k, drawn with a platform-held seed. Every selection writes a `mlops.decision` row:
`decision_kind = select`, full `candidate_set` with signals, `chosen`, `propensity` under the
mixture, `exploration_flag`, `rationale` (one sentence). Ties break on `(created_at, id)`;
scores recompute only when a source changes them.

**Files:** `crates/research-session/src/select.rs`, `crates/ledger` (decision writer).
**Acceptance:** over 40 selections at 0.15, between 3 and 9 are exploration picks; every
selection has a decision row whose candidate set contains every scored item; a `user` item is
the next claim regardless of score; the agenda shown in a digest is the agenda that ran (no
recompute without a source change, asserted).
**Traces:** AGENT-014 P2, §5; §12.3, §12.12 (bypass half). SPEC §4.4, INV-21.

**Built 2026-09-21. `PARTIAL`, and the partial part is which signals have a
source.**

`crates/research-session/src/select.rs` is the scorer: `Signal`, `Provenance`,
`Inputs`, `score`, `select`, `seed_for`, `candidate_set`, `novelty_from_prior`.
The driver measures, scores, draws and logs; `SessionWorker::choose` and
`log_selection` are the two halves.

**What each signal actually reads, because three of the four have a source and
one is named rather than guessed:**

| signal | source | provenance |
|---|---|---|
| `value` | the objective's weight for the item's subject × the strongest source weight on it (§5.1's own table) | `declared` |
| `novelty` | prior hypotheses stating the same question, through the *same* `normalise_statement` `find_prior_hypothesis` uses | `measured{n}` |
| `feasibility` | charter permits the workflow ∧ `Profile::may_run` | `declared`, `{0, 1}` |
| `cost` | median `trials_spent` of finished runs of this workflow on this tier | `measured{n}`, or `not_estimated` below three runs |

**`cost` is in trials, not dollars, and that is a fact about this platform rather
than a shortcut.** `llm_usage.cost_usd` keys on the *auth* session, not the run,
so there is no per-run USD to take a median of. A trial is the budget dimension
the ledger actually counts (INV-1). A dollar figure derived from a list price and
a token count would be a number nobody measured, sitting in the one log every
later analysis reads — which is exactly what `Provenance` exists to prevent.
`feasibility`'s `0.5` rung is not emitted either: it needs the coverage reader
(S-11.10) and the tool breaker (S-11.7).

**The thing that makes this honest is that the row says so.** `Signal` is a
number *and* a `Provenance`, inseparably, and `Scored::not_estimated()` names
which signals contributed nothing. Both ride on the `mlops.decision` row and in
the `selected` event. A reader a year from now can tell a ranking that divided by
a measured cost from one that did not — which a scorer substituting 1.0 silently
would not have allowed.

**Two readings of the spec, both argued rather than assumed.**

*Not everything is percentile-normalised.* §5.1 says every signal is, citing
AGENT-005 §24.3 — but that argument is about **unbounded** signals eating bounded
ones, and it is `value` and `cost` that are unbounded. Normalising `feasibility`
would be actively wrong: a percentile rank turns the lowest value in a slate into
something greater than zero, and feasibility's whole job is that **0 means
impossible**. Same for `novelty`, whose zero means already answered. So the two
unbounded signals are ranked within the agenda and the two bounded ones multiply
as measured. `Scored` carries both scores so the difference is visible, and
`normalisation_cannot_make_an_impossible_item_possible` fails the build if it
changes.

*`k = 1`.* §5.2 says the exploration draw is "uniform over … items outside the
top-k" and then glosses it in plain English as "the item the scorer would not
have picked". Any `k > 1` is a number nobody chose; the gloss is `k = 1`, which
is what is built.

**The draw is replayable, so the propensity is a fact.** `seed_for(session,
event_seq)` and a SplitMix64 written out rather than depended on — a generator
that changed between crate versions would make old decision rows unreproducible.
Greedy gets `1 - share`; each detour gets `share / |detours|`; a `user` item is
not drawn at all and carries propensity 1.0, because attaching a drawn
probability to something that was asked for would be a fabricated number.

**One judgement call in the ledger, recorded as ADR-P11-02.**
`DispatchContext::agent` requires a `delta_practical` and `legacy` records the
absence of one *as a defect of the calling path*. An agenda selection is neither:
it has a propensity (the draw is the platform's and the probability is a fact)
and it has no effect size, because "which question to ask" is not a hypothesis
about a magnitude — §2.2 makes the charter's `delta_practical` a suggestion for
the runs that follow. Forcing it through `legacy` would label a complete
declaration defective and make the flag that marks unaccountable decisions stop
meaning anything. So `DispatchContext::policy_without_effect_size` is the third
case.

**Verified.** `select.rs` carries 13 tests including §12.3's own interval — over
40 selections at 0.15, between 3 and 9 are exploration picks — asserted against
the spec's numbers rather than against whatever this implementation produces.
`crates/invariants/tests/a_selection_is_logged_with_what_it_rejected.rs` holds
the four things nothing else would catch: that `choose` writes a decision row and
not only an event, that the candidate set is built from the slate that was
scored, that the share is read from the session rather than being a constant that
would satisfy every unit test, and that a `user` item is not logged as a draw.

**What is still `PARTIAL`.** §5.4 is implemented as "recompute the whole slate
when any claimable item's score was cleared, reuse the stored ones otherwise" —
which gives stability when nothing changed and consistency when something did,
and is *not* the per-item invalidation the spec's sentence could also be read as.
The `hypothesis_precheck` factor in `novelty` (`proceed_new` 1.0 /
`proceed_as_variant` 0.6) belongs to S-11.4's intake layer and is not applied.
`SelectionPolicy` is a name and a version constant rather than a versioned object
with a conformance reader per field; the weights it would hold are `Source::weight`
and `Objective::weight_for`, both already single-definition and tested, so the
object would today be a second place to write the same four numbers.

### S-11.4 Duplicate work is refused three times — S
`PARTIAL`

At insert: `dedupe_key` (normalised question + subject) — a match to any non-terminal item
extends that item's `source` list instead of inserting. At intake: the child's
`hypothesis_precheck` (AGENT-013 §5.1) — `reject_duplicate` marks the item `superseded{by}`
and charges no trial. At the ledger: the family's trial count reduces `novelty` (S-11.3).
The scorecard reports which layer caught each duplicate.

**Files:** `crates/research-session/src/agenda.rs`, `crates/research-session/src/harvest.rs`.
**Acceptance:** two sessions with overlapping scope produce one item with two sources; a forced
duplicate past the key is caught at intake and superseded with zero trials; the layer is
recorded.
**Traces:** AGENT-014 P3, §4 (dedupe), §6.5; §12.2.

**Built 2026-09-21. Layers one and three were already there; this is layer two.**

§4's table puts layer two in **the child's** `hypothesis_precheck`. It is
performed by the driver *before* the child starts instead, which is stricter
rather than looser: the spec's own words are "charges no trial", and a child
running its own precheck has already opened a conversation and spent tokens
before it can find out. `SessionWorker::intake_check` reads the same two
persisted facts the battery reads and applies no rule of its own — a
`negative_result` whose `retry_condition` is absent (→ `expired`, because blocked
is not duplicate) and a registry hypothesis with the same normalised statement
*and a decided status* (→ `superseded`). `open` and `inconclusive` do not count,
which that column's own schema comment argues for.

`dedupe_layer` and `trials_charged: 0` go on the event, which is what makes P3's
"the scorecard says which layer caught it" possible and what gives `DedupeLayer`
its first caller.

**What is not covered:** the subject half is inert for an item with no subject
and says so on the log rather than passing quietly — the charter form's seeds are
sentences, not subjects, so most seeded items arrive without one. Giving seeds a
subject is a desk change (S-11.13). And the scorecard that *reports* the layer
counts is not built; the events carry what it would read.

## Execution

### S-11.5 Execution: a conversation per item, the brief, the lease, the harvest, failure — L
`PARTIAL`

Claim → one child conversation tagged `session:<id>` → a Research run whose brief
(AGENT-007 §4) includes the charter's preferences, the session's last five verdicts in scope,
and the item's source sentence. `BudgetLease` split from remaining day and total budgets, sized
`estimate_cost × 1.5`, capped by the tier's `max_usd_per_task`; `max_trials_per_task` from the
lease. On `Outcome`: harvest yield, `open_question`s, `Proposal`s (→ *Needs you*; **never**
auto-approved) and next questions; mark `done{verdict}`. A redirecting steer on the child
rewrites the item's question. Failure: re-queue with `attempts += 1`; expire at 2; two children
failing on the same `platform_defect` opens a session-level breaker and pauses.

**Depends on:** S-2.12 (leases, sub-runs), S-10.13, S-10.9, S-10.12.
**Files:** `crates/research-session/src/{execute,harvest}.rs`, `crates/api/src/agent/manager.rs`.
**Acceptance:** a child's lease never exceeds the remaining day budget; three concurrent children
cannot together exceed it; a child exhausting its lease gets the report turn and the session
continues; a harvested proposal appears in *Needs you* and is not approved by anything but a
human; the defect breaker pauses the session with the tool named.
**Traces:** AGENT-014 P4, P6, §6; §12.4.

**The harvest's decision half is built 2026-09-21; execution is not.**
`crates/research-session/src/harvest.rs`: open questions become source 4.3 items
(`OpenQuestionBlocking` when blocking), next questions become 4.8, **each through
`agenda::offer`** — layer one of the duplicate refusal, and there is no second
dedupe anywhere in the module. 12 tests.

**P6 is a type, not a rule written near one.** Proposals come back only as
`AwaitingHuman`, which has no decision field, no constructor that could set one,
and a `digest_record()` with no branch that can write `awaiting: false`. Every
fixture runs through a `harvest_checked` wrapper and a planted-violation test
shows the guard fires. `Ring` is sealed to {1, 2}; rings 0 and 3 are refused at
`Ring::parse`. `Harvest::skipped` names what was dropped, so nothing a run
produced disappears silently.

**One ordering that is load-bearing and has a test:** offers are made *before* the
item is marked done, because `offer` merges onto non-terminal items only. Marked
done first, a run suggesting its own question again would queue up the question it
had just answered.

**One spec reading, argued rather than assumed.** §6.5 routes a run's proposed
next questions to source 4.8, and `agenda.rs` also has a `ProposalFromRun`
variant. Everything a run suggests is emitted as `refinement_followup`, because
`stall::degenerating` keys on `only_from(RefinementFollowup)` — routing half a
session's self-generated work under another source would hide exactly the sessions
that rule exists to catch, and `ProposalFromRun`'s weight of 1.0 would *up*weight
self-generated work against §5.1's table, where follow-up is the lowest.

**What is not covered:** leases (§6.3), the child conversation and brief, reading
a real dossier into the harvest record, and §6.6's session-level
`platform_defect` breaker — that last is a rule about the session and cannot be
decided by a function that sees one run. `execute.rs` does not exist.

### S-11.6 Schedule windows and the day budget — M
`PARTIAL`

Outside a window, or past the day budget: no new claims; live children finish the current step
and suspend at the boundary (AGENT-010 §10.1's per-run drain), holding no connection and
spending no tokens. The next window resumes suspended children **before** claiming anything
new. Windows are `{days, start, end, tz}`; the driver re-arms on the window boundary. The day
budget is pre-filled from the desk budget policy (platform default $25/day), never required.

**Files:** `crates/research-session/src/schedule.rs`, `crates/api/src/session_driver.rs`.
**Acceptance:** closing the window mid-step suspends at the boundary with no lost effect and
zero tokens until the next window; the next window resumes before claiming; a day budget hit
behaves identically and resets at the window; a charter created without a day budget carries
the policy default.
**Traces:** AGENT-014 §2.1 (budget defaults), §7 (rows 1–2); §12.5, §12.10 (budget half).

**The calendar is built 2026-09-21; the drain is not.**
`crates/research-session/src/schedule.rs`: `OpenHours::parse(&Schedule)` is the
only way to get a readable schedule and it is **fallible**, which is the whole
point — `is_open_at`, `next_open_after`, `next_open_on_a_later_day` (§7 row 2's
"next day's window", which is not the same as the next window), `closes_after`
(the drain boundary row 1 needs) and `openness()`. 15 tests.

**An unknown timezone is refused by name and there is no path to UTC.** A fence
that silently moved is not a fence, and the test proves the refusal is worth
something by showing the same window read as UTC *is* open at an instant New York
fences off. `00:00` as an end is refused too, as is any `end <= start`: a wrapped
Friday 22:00–02:00 would run in Saturday's small hours while the charter said
Friday, and the error names the two-window remedy. `desk_charter`'s grid already
emits `24:00` for the same reason, so the two agree.

DST is handled and asserted: a repeated hour is inside the window both times, an
hour that never happens opens no window, and a window straddling the gap opens at
the jump. The invariant that stops a driver busy-looping —
`next_open_after` never returns an instant `is_open_at` calls closed — is swept
over 5 schedules × 5 start points × 48 hours and **asserts it checked > 500
instants**, so it cannot pass by sweeping nothing.

**Wired at the door the same day.** `routes::desk_charter::translate` now calls
`OpenHours::parse`, so a typo like `Amercia/New_York` is refused at the charter
form. It used to be written into every window unchecked and failed days later, on
a session that had been running in UTC.

**What is not covered:** everything driver-side — suspending a live child at the
boundary and resuming it before claiming anything new, and the day-budget
accounting and its reset. §7 row 2's "the day" is also undefined for a schedule
mixing zones or for the always-open default, and the calendar returns `None`
rather than inventing a midnight in a zone nobody named.

### S-11.7 The couplings: kill switch, tier, platform defects — M
`NOT STARTED`

A `monitor.events` kill-switch trip pauses every running continuous session within one event
(`paused{kill_switch}`), children suspend at the boundary, and no `deploy_paper` proposal can be
created (S-10.9's refusal); reset resumes and the digest records both. A withdrawn or degraded
profile pauses `{tier}`. The defect breaker (S-11.5) pauses `{platform_defect}` and files the
`platform_defect` entry.

**Depends on:** S-7.10 (monitor events), S-4.13 (tier measurement).
**Files:** `crates/research-session/src/couplings.rs`.
**Acceptance:** tripping the switch pauses all continuous sessions within one event and a
`deploy_paper` proposal is refused while paused; reset resumes them; withdrawing a scorecard
pauses a session on that profile; the digest carries each transition.
**Traces:** AGENT-014 §7 (rows 8–10); §12.6. COMP-007 §6.

### S-11.8 The stall detector — M
`PARTIAL`

After every finished run, over the last `K = max(6, 20%)`: barren streak (≥ 4 of K, or 3
consecutive with `knowledge_units = 0`); degenerating (all K from `refinement_followup` with no
established finding and no new class); circling (≥ 2 `proceed_as_variant` of one family with
one failure class); cost drift (> 3× the session's own median, once ≥ 2K runs). Any → `stalled`
with a diagnosis naming which, the items, and a suggested charter amendment — never a lower
`delta_practical`. Nothing runs until a human acts.

**Depends on:** S-10.13.
**Files:** `crates/research-session/src/stall.rs`.
**Acceptance:** six barren runs trip `barren_streak` naming the six; refinement-only items trip
`degenerating`; a stalled session claims nothing; the suggestion never touches `delta_practical`
(asserted on every fixture).
**Traces:** AGENT-014 P5, §7.1; §12.7.

**Built 2026-09-21.** `crates/research-session/src/stall.rs`:
`detect(&[FinishedRun]) -> Option<Diagnosis>`, a pure function over a slice with
no database in it, which is why it tests. All four §7.1 rules with
`K = max(6, ceil(20%))`; the diagnosis names which rule, **every** item involved
and a suggested amendment. 25 tests.

**"Never a lower `delta_practical`" is enforced three ways, not asserted once.**
`Amendment` is a closed enum with no variant that can carry a number; the guard
is an exhaustive `match` that will not compile past a new variant; and **every
test in the module goes through a `detect_checked` wrapper**, so "asserted on
every fixture" is a property of the module rather than of whoever wrote the last
test. `the_guard_against_lowering_the_bar_would_catch_a_violation` plants a
violation and asserts the guard fires — a guard nobody has seen fail is a guard
that might be matching nothing.

**One spec finding, written up as Q-S11-02 rather than resolved quietly.** §7.1's
cost-drift rule, read literally, cannot fire at `2K` runs — the exact point its
own gate opens — because a window inside its own median can never exceed 3× it.
Proved over three orders of magnitude and kept as a test. The build takes the
median over the runs *before* the window; the sentence needs deciding.

**What is not covered:** the caller. `detect` needs `FinishedRun`s carrying
knowledge units, failure class, family and cost, and `Folded::finished` is
`item → verdict` — so the driver must join the fold to `run_yield` and the agenda
table. **`AgendaItem` has no `family`**, and two of the four rules group on one,
so it must come from the run's hypothesis. Nothing appends `session_stalled` yet.

### S-11.9 Autonomy, the frontier-only rule, and the `judgement` threshold — S
`NOT STARTED`

`supervised | notify | autonomous` governs *starting* only; **default `notify`**. Continuous
may be switched on only for a **frontier** profile that is `measured` and passes `judgement`
for every workflow in `permitted_workflows` (a failing workflow is removed from the permitted
set with a note); a non-frontier profile is refused at the boundary with the profile named
(revisit trigger in AGENT-014 §8). `autonomous` is refused (not ignored) below the profile's
`judgement` threshold. In every setting `SealedHoldout` and `Promotion` ask, `Spend` asks above
the ask line, and ring 3 does not exist.

**Depends on:** S-10.16.
**Files:** `crates/research-session/src/charter.rs`, `crates/api/src/routes/research_sessions.rs`.
**Acceptance:** a charter on a local profile is refused with the profile named; `autonomous`
below threshold is refused at the boundary; a new session defaults to `notify`; under
`autonomous` a `Promotion` action still suspends for a human (adversarial).
**Traces:** AGENT-014 P6, §8; §12.10.

## Sources that watch the world, and the endless setting

### S-11.10 Observation subscriptions: regime, coverage, monitor — M
`NOT STARTED`

Source 4.7: an instrument-profile regime flip (S-4.7's profiler) → one `investigate` item per
flip; new data coverage in scope → `investigate`; a monitor `divergence` or `scaled_down` event on
a strategy in scope → `diagnose`. Rate-limited per subject. This is "market observations into
experiments".

**Depends on:** S-4.7, S-7.10.
**Files:** `crates/research-session/src/sources/observe.rs`.
**Acceptance:** a regime flip inserts exactly one item per flip across three profiler runs; a
monitor `scaled_down` on an in-scope strategy inserts a `diagnose` item citing the event; an
out-of-scope strategy inserts nothing.
**Traces:** AGENT-014 §4.7; §12.8 (observation half).

### S-11.11 Replenish — the endless setting — M
`NOT STARTED`

With `replenish = true` ("keep generating questions" in the desk) and a drained agenda, the
driver claims a synthetic `survey` item over the charter and everything in scope; the
`SurveyReport.proposed_questions[]` become items with the survey's value estimates and
`source = replenish`. With `replenish = false` the session is `complete`. A replenish that
proposes nothing new (every proposal deduped) counts toward the stall detector's barren streak.

**Depends on:** S-10.14 (`survey` skeleton).
**Files:** `crates/research-session/src/replenish.rs`.
**Acceptance:** a drained agenda under `replenish = true` runs a `survey` and its proposals
appear as scored items; under `false` the session completes; a fully-deduped replenish
increments the barren streak.
**Traces:** AGENT-014 §2.1 (`replenish`), §4.9, §7; §12.9. ADR-0037 §2.

## The human's side

### S-11.12 The digest — M
`PARTIAL`

Written by the platform every `review_every`: verdicts (not titles), findings established and
refuted, negatives with new classes, questions opened/closed, contradictions found/resolved,
proposals awaiting, budget and cost per knowledge unit, the next five items with scores and
why, every pause and its reason. Each line resolves on click. Rendered as a card in the
session's journal conversation. Also written on stall, stop and complete.

**Depends on:** S-10.13, S-6.24 (the same renderer as *Since you were away*).
**Files:** `crates/research-session/src/digest.rs`, `frontend/src/components/desk/cards/Digest.tsx`.
**Acceptance:** a digest for a fixture period lists every finished run's verdict, every awaiting
proposal and the cost per knowledge unit; every id resolves (AGENT-012 P9's test); a stall
writes one immediately.
**Traces:** AGENT-014 P7, §2.4; §12.11.

**Built 2026-09-21, the object and not yet the cadence.**
`crates/research-session/src/digest.rs`: `write(&DigestInput) -> Digest`, pure,
**no clock and no model handle** — which is where P7 is true rather than
promised. Covers §2.4: runs by verdict, findings established and refuted,
negatives marked when the class is new, questions opened and closed,
contradictions found and resolved, proposals *still awaiting* only, budget with
cost per knowledge unit, the next five items with scores and why, and pauses.
15 tests.

Two of §2.4's rules are types rather than conventions. `Line` holds a
**non-optional** `Ref`, so a line without an id cannot be constructed — which is
what "every line resolves on click" has to mean; `Digest::unresolvable()` catches
the one hole a type cannot (an id that arrived empty) and is asserted empty on a
15-line fixture. And `RunRecord` has **no title field at all**, which is §2.4's
"verdicts, not titles" made unrepresentable.

**What is not covered, and it is most of the task's wiring:** the `review_every`
cadence and the write-on-stall/stop/complete triggers (the driver's, in
`crates/api`), persistence to `session_digests`, and
`frontend/src/components/desk/cards/Digest.tsx`. Everything the cadence needs is
computable from `Folded::last_digest_seq`, `Folded::runs_started` and
`ReviewEvery`.

**One trap for whoever wires it.** `BudgetSummary.spent` and its cost-per-
knowledge-unit line are rendered to a human beside a day budget denominated in
**dollars**, and `cost` in this crate is in **trials** (`select.rs` scores it that
way because `llm_usage.cost_usd` keys on the auth session, not the run). The
stall detector survives this because drift is a ratio and the unit cancels; the
digest does not. Pass real USD or change the digest's wording — and when per-run
USD lands, `select.rs`'s `Provenance` machinery is the pattern to copy.

### S-11.13 The desk: the toggle, the charter form, the Agenda and Charter tabs, the rail, home — L
`NOT STARTED`

Two options in the composer — Chat / Research — and a **Continuous** toggle under Research.
Toggle on: the question field becomes the **charter form** (scope chips, objective picker,
seeds, permitted workflows, exclusions, budget pre-filled from policy, schedule grid, autonomy
defaulting to `notify`, *keep generating questions*, `review_every`; a non-frontier profile is
refused inline). The session's conversation is the journal (digest cards, `/agenda add`,
`/pause`, `/resume`, `/charter`); its right panel gains **Agenda** (board: proposed · queued ·
running · done · vetoed; drag = `user` priority event; veto; add; score components and source
badges per card; exploration pick marked; running cards show the child's stage strip) and
**Charter** (versioned, amendments listed) tabs; the header shows a *continuous* badge and the
day budget bar. Child conversations group under the journal in the rail and carry a chip back.
Desk home: a continuous session is one row in *Running now* with next item, agenda depth and
day budget; a stalled one is in *Needs you* with its diagnosis and resume · amend · stop.
**No page is called Programs; the word does not appear in the UI.**

**Depends on:** Phase 6, S-11.12.
**Files:** `frontend/src/components/desk/{Composer,CharterForm}.tsx`,
`frontend/src/components/desk/panel/{AgendaTab,CharterTab}.tsx`, `DeskHome.tsx`,
`ConversationHeader.tsx`, `ConversationRail.tsx`.
**Acceptance:** toggling Continuous swaps the question field for the charter form and back
without losing typed constraints; a session's state, next item and budget are readable from
*Running now*; a veto on a queued item prevents its claim and shows in the digest; a reorder
writes a `user` priority event; a grep of the desk source finds no user-visible "Program";
every id resolves; both themes, both densities, 375 px.
**Traces:** AGENT-014 §9; §12.11, §12.12. AGENT-012 §2, §2b, §3.3 (amended), §19.10.

### S-11.14 Session tools and the API — M
`PARTIAL`

Read-only, in the `lifecycle` namespace (9 → 12; `research` untouched at 16):
`lifecycle.get_session`, `lifecycle.list_agenda`, `lifecycle.get_digest`. API per AGENT-014
§10 under `/api/research/sessions…`: list/create/get, `pause|resume|stop`, agenda
list/add/`approve|veto|prioritise`, charter `PUT`, digests, SSE events. Agent tokens hold
`research_sessions:read` only; the write path into an agenda from a run is
`lifecycle.open_question`/`propose`, which the driver harvests.

**Files:** `crates/mcp-server/src/tools/lifecycle.rs`, `crates/api/src/routes/research_sessions.rs`.
**Acceptance:** an agent token cannot call any session POST (adversarial); the three tools are
in the `InternalAgent` profile and the namespace cap test passes with `lifecycle` at 12 and
twenty-two namespaces; SSE delivers agenda transitions to an open journal within one event.
**Traces:** AGENT-014 §10; AGENT-006 §2.1 (amended), §10.11.

**Built 2026-09-21, API and tools. `PARTIAL`.**

`crates/api/src/routes/research_sessions.rs`: list, create, get, `pause` /
`resume` / `stop`, agenda list/add, `approve` / `veto` / `prioritise`, charter
`PUT`, digests, and SSE over the log keyed on `seq` so a reconnect resumes
exactly where it stopped. `resume` also **submits the driver job** — without that
the log would say `running` and nothing would ever claim, which is this
repository's dominant defect shape reached through a job queue.

Every mutating route asks for `web:full`, a scope `NEVER_FOR_AGENTS` refuses to
attach to any project-bound session and migration 0038's trigger refuses again at
the row. `every_mutating_handler_asks_the_same_question` scans this file and
fails the build if a new write handler is added without the check.

Three read tools in the `lifecycle` namespace: `get_session`, `list_agenda`,
`get_digest`, all `.internal()`. Each takes an **optional** `session_id`: with it
omitted the platform answers from the calling run's own attribution via
`GET /api/research/sessions/current`, which reads the run id out of the token's
minted label the way `agent::audit::principal_for` does — so a run does not have
to be told an id it could get wrong, and cannot name another session.

**The desk was already calling this, and had been for weeks.**
`crates/api/tests/the_desk_calls_routes_that_exist.rs` carried four phase-11 rows
in its `UNBUILT` list — `/api/research/sessions`, `…/{}/agenda`,
`…/{}/agenda/order`, `…/{}/agenda/{}/veto`. The composer has posted a charter to
the first since it was written. They are removed, and the test fired on its own
the moment three of the four became served, which is what it is for.

Closing that gap meant making the two sides agree rather than only making the
path exist, and three mismatches came out of it:

* the client posts `{mode, continuous, question?, charter?, workflow?}` with **no
  conversation id**, because the composer is starting the conversation and the
  session at once. `conversation_id` is now optional and one is opened when it is
  absent — §2.1's "the conversation IS the session" read forwards. `model` was
  added to the client's body so §8's frontier rule is enforced on the server too;
  a rule enforced only in the browser is a rule a `curl` walks around;
* the charter form is **flat** (chip lists, a 7 × 24 grid, a day budget beside a
  `max_runs`) and `research_session::Charter` is §2.2's nested shape.
  `crates/api/src/routes/desk_charter.rs` translates, with the decisions in it
  tested: the grid becomes *contiguous windows* rather than 168 one-hour ones, an
  empty grid is always-open rather than never, and an unparseable
  `delta_practical` is **refused rather than dropped** — it is an effect size the
  runs under the charter are suggested to register against, and silently
  discarding what the user typed is what makes a suggestion meaningless;
* the board has **five columns** and an item has **nine statuses**.
  `ItemStatus::board_column` is the projection, held total by a test that also
  asserts every column is reachable — a column nothing can be in renders empty
  forever and teaches the reader it is broken. The response carries both, so a
  card can say `expired` while sitting in `done`.

And one crash that would have happened on the first continuous session anybody
started: `AgendaTab` read `i.score.value` unconditionally, and the first server to
serve it has no scorer (S-11.3). It now renders "not scored yet" — not a zero,
which would read as *worthless* — and `agenda-board.test.tsx` covers it.
`POST …/{id}/agenda/order` is built: a drag writes a `user` source per item, which
is the same bypass the scorer already honours, rather than a rank column that
would be a second ranking to keep in step.

**Three things this does not yet do.** The adversarial "an agent token cannot
call any session POST" is asserted by a source scan over the handlers, not by an
HTTP test with a real service token — weaker, and worth upgrading. `research` is
untouched at 16 as the task asks, but **`lifecycle` is at 9, not 12**: the task's
"9 → 12" counted nine pre-existing lifecycle tools and there are six
(`preregister`, `hypothesise`, `design`, `propose`, `find_prior_hypothesis`,
`open_question`). Three named in AGENT-013 §8 are not built; which three is a
phase-10 question, and the count is recorded here rather than the tools invented
to make it match. The SSE half of the acceptance — "delivers agenda transitions
to an open journal within one event" — has no journal to deliver to until S-11.13.

---

## Exit criteria

- A continuous session survives a killed driver with one child and an identical fold; a bounded
  Research run is the same object with one item.
- The next question is chosen by a declared policy with a forced exploration share, and every
  choice is a decision row with its full candidate set.
- No question is run twice: key, intake, ledger — and the scorecard says which layer caught it.
- Budget is spent only through leases; schedule windows suspend and resume without loss.
- A tripped kill switch pauses every continuous session; a stalled session says why and waits.
- Continuous is frontier-only, defaults to `notify`, and its day budget is pre-filled.
- Endless means `replenish`; with it off, the session completes.
- Returning after a week costs one digest; the user only ever saw Chat, Research and a toggle.
