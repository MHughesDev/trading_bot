# Phase 11 — Continuous research: Research with the toggle on

**Completion: 71% (10 / 14 tasks), plus 4 `PARTIAL` — 2026-10-04, F-SESSION, re-counted by track VERIFY: 10 / 14 `DONE` (S-11.1, 3, 4, 5, 6, 7, 8, 9, 10, 11), of the `PARTIAL` three (S-11.2, 12, 14 — each waiting on a desk click-through or an HTTP-level agent-token test), S-11.13 is not this track's; was 2 / 14 plus 12 `PARTIAL`** (S-11.3 and S-11.8 closed 2026-10-03; S-11.13 moved to `PARTIAL` the same day and is not this track's) — before that no task was closed and eleven had a built, tested crate half. **Five of those eleven read `NOT STARTED` until 2026-09-22**, over modules of 443–1,063 lines carrying 11–21 tests each; the markers were corrected against the tree, not against a report. The reason none is `DONE` is one sentence: `api::session_driver::SessionWorker` calls `schedule`, `harvest`, `stall` and `digest` and nothing else, so `sources::ingest`, `couplings::evaluate`, `charter::admit`, `replenish::on_drain` and all of `execute` are complete and unreached.

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

> **The modules' six gaps, 2026-09-21 — four closed, two landed separately
> because they crossed a crate boundary mid-flight.**
>
> Wiring the four pure modules exposed six places they could not say what their
> caller needed, and every one was the same rule: **an absent measurement is
> never a zero.**
>
> `digest::SpendUnit { Trials, Usd }` puts the unit **in the sentence** —
> `12.00 trials spent over 3 runs` — with `as_str` matching `0139`'s CHECK word
> for word and a test pinning the two together. `EventKind` gained
> `ProposalAwaiting` and `Note`, so a proposal awaiting a human is no longer
> filed as an agenda approval and a plain note is no longer filed as a merge;
> the P6 guard is held by a test that nothing about a proposal enters `Folded`
> and that `proposal_awaiting` is the **only** word in the vocabulary naming one.
> `stall::FinishedRun.family` became `Option`, and the rules moved with it:
> `circling` skips an unfamilied run rather than grouping on `None`, and
> `sole_value` returns nothing if *any* run in the window lacks a family — so an
> `ExcludeFamily` amendment can never name a family the window did not record.
> `Precheck::NotRecorded` exists and is now the **default**, because with
> `hypothesis_precheck` filing `blocked | duplicate | new`, absence is the common
> case and a fixture whose neutral state is a verdict nobody filed flatters the
> rules.
>
> **Two were refused and reported instead**, because another agent was compiling
> against this crate at the time: changing a struct variant breaks every
> exhaustive pattern, and changing a field's type breaks the caller mid-task.
> Both landed afterwards with both sides moving together — `Insert::Merged` now
> carries **the refs the merge added** (not everything offered: the caller
> appends with `||`, so a ref the item already held would duplicate), which is
> what stops a harvested question merging onto a card that can no longer open the
> memory row that wanted it.
>
> **One finding about the spec, and it is the root of the units trap.** §5.2
> gives the scoring cost term in **USD floored at $0.10** and §6 sizes the lease
> in USD, but `llm_usage.cost_usd` keys on the auth session rather than the run,
> so **nothing on this platform can price a single run** — which is why `select`
> scores trials. A floor of `$0.10` applied to a number counted in trials is not
> a constraint anyone can evaluate, and §2.4's "budget spent and cost per
> knowledge unit" then names no unit at all, which is how a trials figure came to
> render beside a dollar bar. `SpendUnit` makes the question answerable without
> answering it; `Usd` exists precisely so a digest written the day a run can be
> priced can say so.

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
`DONE`

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

**2026-10-03 — the lease half of acceptance 1 is a schema fact.** `migrations/0160_session_leases.sql`: one row per `(session, item, run)`, a partial unique index allowing **one live lease per agenda item**, a trigger that lets a lease be released once and refuses any other UPDATE or a DELETE, `GRANT SELECT, INSERT, UPDATE` to `app_role` and DELETE revoked. The driver writes the lease (`SessionWorker::record_lease`, `ON CONFLICT DO NOTHING`) between `start_child` and `run_started`, so a driver restarted in that window re-derives the same child (ADR-P11-01) and writes nothing the second time; it releases it when the child ends. Validated on a scratch database on 5433 (all migrations through 0160) and `crates/api/tests/pg_session_leases.rs`, **4/4 under `SET LOCAL ROLE app_role`**: `a_restarted_driver_records_one_lease_for_the_one_child`, `three_live_leases_are_summed_and_a_finished_one_is_not`, `a_lease_is_committed_in_full_and_released_once_and_never_deleted`, `a_lease_that_buys_nothing_or_names_no_unit_is_refused`. Still `PARTIAL` on one clause: acceptance 4 (a bounded run has a session row with one item) is wired through `AgentManager::ensure_session` and covered at the store level only — no test drives a real turn through it.

**Final marker `PARTIAL`** (wave 2 re-check): acceptance 1 (kill the driver between claim and `run_started`) holds by the derived conversation id, the conditional claim and now the one-live-lease-per-item index, with restart tests; acceptances 2 and 3 are proven against Postgres; acceptance 4 (a bounded run has a session row with one item) is wired through `AgentManager::ensure_session` and tested at the store level only — no test drives a real turn through it.


**2026-10-04 (F-SESSION) — the real-turn test exists, and it found why clause 4 has never held in production.** `crates/api/tests/a_session_runs_real_turns.rs` drives real turns through a real `AgentManager`, run loop, job store and `SessionWorker` against a fake Anthropic server (a credential's `base_url`; only the model is replaced). Run against the scratch database it fails at the first assertion with `the run ended \`offered\``: **`AgentManager::send_turn` cannot start a Research run.** Its demand is `match opts.mode { Some(Chat) => SingleLookup, _ => CHAT_DEMAND }` and `CHAT_DEMAND` is `SingleLookup` since ADR-S10-02, so `mode: Some(Research)` (and `None`) both launch a **Chat** run: ring 0, no trial-spending tool, and `launch_inner` calls `ensure_session` only for `Mode::Research`, so **no session row is ever opened by the front door**. The only code that launches `MultiStep` is `start_offered_research`. Consequences beyond this clause: every child a continuous session starts is a Chat run that ends in an offer card (S-11.5's lease buys nothing), and the doc on `TurnOptions::mode` ("`None` is Research") is false. The session driver (`start_child`) and `routes::research_sessions::create` now say `mode: Some(Research)` explicitly through `send_turn`, which is correct whatever the manager does. **The one-line fix is in `crates/api/src/agent/manager.rs` (RUNTIME's, not this track's):** in `send_turn`, `Some(harness::state::Mode::Research) => harness::hardware::TaskDemand::MultiStep`. Until it lands the two tests in that file fail (`a_bounded_research_run_is_a_session_with_one_item_through_a_real_turn`, `three_children_run_together_and_their_leases_never_exceed_the_day_in_trials`); they are written to pass after it and are unverified past the first assertion. Clauses 1-3 unchanged and held.


**2026-10-04, after the manager fix — `DONE`.** The coordinator applied the one-line `send_turn` fix (`Some(Mode::Research) => MultiStep`). Acceptance 4 now holds through a real turn: `a_bounded_research_run_is_a_session_with_one_item_through_a_real_turn` runs the front door against a fake provider and finds one session row (`continuous = false`), one agenda item, a clean fold with `runs_started = 1`, the run attributed to its session and item, and a second turn in the same conversation opening no second session. Acceptances 1-3 were already proven. `crates/api/tests/a_session_runs_real_turns.rs` (2 tests, both pass, against the scratch database with a fake Anthropic server): `a_bounded_research_run_is_a_session_with_one_item_through_a_real_turn`, `three_children_run_together_and_their_leases_never_exceed_the_day_in_trials`.

### S-11.2 Memory-fed sources: open questions, retry conditions, contradictions, stale findings, follow-ups — M
`PARTIAL`
`sources/memory.rs` (443 lines, 11 tests) covers §4.3 open questions, §4.5
contradictions and §4.6 stale findings; `sources/ledger.rs` (539 lines, 11
tests) covers §4.4 retry conditions. `sources/mod.rs` carries `World`, `ingest`,
`offer_all` and `admitted`, with `Fired` **three**-valued — `NotEvaluable` is
not a `No`.

**Re-checked 2026-10-03: the "no production caller" sentence that stood here was stale.** Commit 42739eca wired `sources::ingest_for` into `SessionWorker::ingest_subscriptions` on every iteration (including a drained board, b6a47d86); `tests/the_session_driver_calls_the_modules_it_was_given.rs` holds the call. `PARTIAL` still, for two reasons outside this crate: §4.5's contradiction *pair* has no store (`agent_runs.contradictions_surfaced` is a count), so that subscription is asked nothing and says so on the log; and "opens in the desk" is S-11.13.


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

**Final marker `PARTIAL`** (wave 2 re-check): the retry-condition and stale-finding clauses are reached in production; the contradiction clause is not (no pair store), and "opens in the desk" is S-11.13.


**2026-10-04 (F-SESSION) — the contradiction pair has a store and the source is produced.** Migration `0161` `session_contradictions` (ordered pair, question, found/resolved, resolve-only trigger, no DELETE for `app_role`); `session_contradictions::refresh` materialises a pair from `agent_memory.entry.conflicts_with` (live both sides, one row whichever entry lists the other) and resolves it when either side is archived or superseded; `SessionWorker::world` calls `refresh` then `open_pairs` every iteration, so source 4.5 inserts a `diagnose` item carrying both entry ids. `GET /api/research/contradictions` serves the pairs the card opens, and the digest carries pairs found/resolved in the period (new `DigestInput::contradictions_found/resolved`, `a_pair_found_in_the_period_is_a_line_that_resolves_to_the_pair_and_not_to_a_run`). Proven against Postgres as `app_role`: `a_conflict_becomes_one_pair_whichever_entry_lists_the_other_and_resolves_when_one_leaves`. **Still `PARTIAL`:** "opens in the desk" is a click-through the desk track owns; the data and the route exist.

### S-11.3 Scoring, exploration, and the decision row — M
`DONE`

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

**2026-10-03 — the last two unwired pieces have callers, and the fourth clause is asserted.** `measure` now calls `select::novelty_from(precheck, n_prior)` with the item's last `hypothesis_precheck` verdict (`mlops.workflow_verdict` over runs attributed to the item; `precheck_for_novelty` keeps an absent verdict `NotRecorded` instead of `proceed_new`), and each candidate on the decision row and the `selected` event carries its `PrecheckFactor`, with the note in `not_estimated` when none was recorded (`the_novelty_precheck_reads_an_absent_verdict_as_absent`). `SelectionPolicy::current().nonconformances()` is called at the top of `SessionWorker::run`; a drifted policy refuses the job by name rather than ranking under it. §5.4's rule is `slate_is_stale` and is asserted (`the_slate_is_rescored_only_when_a_source_cleared_a_score`). All four acceptance clauses are now met — the 40-selection interval (`select.rs`), the full candidate set on the decision row (`a_selection_is_logged_with_what_it_rejected`), the `user` bypass, and no recompute without a source change. **Left `PARTIAL` deliberately**, because two spec signals are not what §5.1 says: `cost` is in trials (no per-run USD exists) and feasibility's 0.5 rung is not emitted (no coverage reader). Both are recorded on every row through `Provenance`; neither is an acceptance clause, so this is the task closest to `DONE` and the call is the orchestrator's.

**Final marker `DONE`** (wave 2 re-check, after the three wirings above). All four acceptance clauses hold with production callers: the 40-selection interval (`select.rs`), the full candidate set on every decision row, the `user` bypass, and no recompute without a source change (`slate_is_stale`). Two signals are narrower than §5.1 writes — `cost` is in trials and feasibility's 0.5 rung is not emitted — and both are on every row through `Provenance`, which is the spec's own requirement that an absent measurement is never a zero.

### S-11.4 Duplicate work is refused three times — S
`DONE`

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

**2026-10-03 — the scorecard has a reader.** `DedupeScorecard::from_log` builds P3's card from the events the driver already writes (`agenda_item_added` = inserted, `agenda_item_merged` = layer one, `agenda_item_superseded/expired` with `dedupe_layer` = layer two); layer three stays `NotReported` because the ranking lowers a score and refuses nothing it could count. `SessionWorker::digest_input` puts it on every digest (`Digest::dedupe`, stored with the body). Tests: `the_scorecard_read_off_the_log_says_which_layer_caught_each_duplicate`, `the_dedupe_scorecard_rides_on_the_stored_digest`. **Still `PARTIAL`:** acceptance 1 says *two sessions* with overlapping scope produce one item with two sources, and the dedupe key and its partial unique index are per session — cross-session dedupe is not built, and is a design question (two sessions can have different charters for the same question) rather than an oversight.

**Final marker `PARTIAL`** (wave 2 re-check): clauses 2 and 3 are met; clause 1's *two sessions with overlapping scope produce one item with two sources* is not built, the key being per session.


**2026-10-04 (F-SESSION) — clause 1 built: `DONE`. ADR-P11-07.** Two continuous sessions of one user whose offers share a dedupe key now produce **one item with two sources**: `session_dedupe::offer` (called by `SessionWorker::place`, the one door for seeds, subscriptions, survey proposals and harvested questions) merges a second session's offer onto the first live session's live item, extends its `sources`, records the sponsor in `agenda_item_sponsors` (0164), writes `agenda_item_merged{{cross_session}}` on the offering log and a `sponsored_by` note on the owner's, and tells the sponsor how the item ended when it settles. Proven against Postgres under `app_role`: `two_sessions_with_one_question_produce_one_item_with_two_sources` (sources `[open_question, retry_condition_met]`, the second session holds no copy, told once), `a_stopped_sessions_board_is_not_a_place_a_question_is_answered`; mutation (drop the same-user condition) fails both. *Rejected:* copying the item into the second session (the question runs twice, which is what this task forbids) and moving it (takes a card off a board its owner is watching). Limits chosen: same user and same key only; a `user`-sourced offer is never rerouted (it is an instruction to *this* session); only running/paused/stalled sessions and non-terminal items.

## Execution

### S-11.5 Execution: a conversation per item, the brief, the lease, the harvest, failure — L
`DONE`

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

**2026-10-03 — the lease reaches every child, and the brief reaches the child.** Three changes, each removing an inert path:

* **Every lease on a fresh platform was refused.** A blank trial field stored `trials_total = 0`, the per-task cap was undenominated in trials, and a workflow with < 3 finished runs had no estimate — which `lease` read as *unread*. No estimate without runs, no runs without a lease: no continuous session could start a child. Now `execute::per_task_bound` is §2.1 read backwards (`trials_total / max_runs`, or `DEFAULT_TRIALS_PER_TASK` for a blank row), creation fills a blank total (`SessionBudget::with_default_trials`), and an absent estimate is `Observed::Is(None)` — real caps size the lease and it records `estimated: false`. An unread estimate still refuses; no real cap still refuses (`a_workflow_nobody_has_run_yet_is_leased_by_its_real_caps_not_refused`, `a_blank_trial_field_is_bounded_by_the_per_task_cap_and_not_refused`, `an_unmeasured_cost_is_read_off_its_provenance_and_not_off_its_number`).
* **Committed leases are counted.** `LeaseRequest::in_flight` and `day_budget` take the sizes of unreleased `session_leases` rows (0160); §12.4's "three concurrent children cannot together exceed" is a sum (`three_concurrent_children_cannot_together_exceed_the_day_budget`, and `three_live_leases_are_summed_and_a_finished_one_is_not` against Postgres). A live child with no recorded lease still refuses a sibling.
* **§6.2's brief is in the child's first turn.** `execute::brief` had no caller; `SessionWorker::first_turn` now sends the question followed by `Brief::first_turn` (why now, standing constraints, the last five sibling verdicts in scope) through `send_message` (`the_first_turn_is_the_question_then_the_brief_and_omits_what_is_empty`).

**ADR-P11-03 (2026-10-03) — the per-task trial cap is the platform's `DEFAULT_TRIALS_PER_TASK = 40` until a tier declares one.** §2.1 fills `trials_total` from "the tier's `max_trials_per_task`", which no tier declares. *Rejected:* leaving it blank (every lease refused, above) and deriving trials from `max_usd_per_task` (needs a per-run price, which does not exist). Taken: one declared constant in `charter.rs`, pre-filled and editable exactly like the $25/day, replaced the day a tier declares its own.

**Still `PARTIAL`:** the lease is **not enforced on the child** — `AgentManager::send_message` takes no trial cap, so "a child exhausting its lease gets the report turn" depends on a manager change (another track); `max_concurrent_runs > 1` offers slots but the driver still starts and awaits one child per iteration; the §6.6 breaker's `platform_defect` memory entry is not written (S-11.7); and §6.4's redirect has a handler (`routes::research_sessions::redirect_item`) that is **not yet routed** — the `routes/mod.rs` line is with the orchestrator.

**2026-10-03, wave 2 — the lease is enforced, the boundary is performed, the breaker files its entry.** RUNTIME's `AgentManager::send_message_capped`, `drain_run` and `resume_drained` are wired into `session_driver.rs`:

* **`send_message_capped`** takes `lease.max_trials_per_task()` (a lease in dollars is uncapped and says why). `run_finished` now carries `trials_spent` and `lease_exhausted` — `None`, never 0, when either side is unmeasured.
* **`drain_run` / `resume_drained`.** `SessionWorker::await_or_drain` asks `execute::plan` every poll whether a `Drain` is due (closed window, spent day, kill switch, platform-defect breaker; a human **Pause** counts as a closed window; a degraded tier does not drain) and stops the child at its next step boundary — still `running`, nothing killed (INV-17). The drain is a `note{drained_run}` on the log, the lease stays committed, and the next iteration puts the child in `Situation::suspended`, so `execute::Step::Resume` — returned only once the window is open and the couplings clear — resumes it **before** claiming anything new (`resume_one` → `adopt` → the same `settle_child` a freshly started child uses). `perform_drains` does the same for live children nothing is awaiting, and an orphaned live child (driver restarted mid-run) is adopted rather than lost.
* **The job no longer exits on a closed window or a coupling pause.** `Step::Wait` sleeps (up to a week, then a human's resume submits a fresh job): nothing else re-armed the session, so before this a window that closed ended the job and nothing ever claimed again. A pause a coupling caused (`kill_switch`, `tier`) is **lifted automatically** when it clears (`session_resumed{auto, after}`); a human's pause and `platform_defect` are not. The digest's *Paused* section now carries the reset and both halves of the boundary (`a_pause_line_is_written_for_every_kind_that_stops_the_session`).
* **`platform_defect` memory entry** (`defect_entry`, filed in `hold` before the pause is appended, idempotent id `pd_<last run>`): the claim is `DefectBreaker::entry_claim`'s own sentence, the subject is the failing tool, and the provenance is the *rule's* — `continuous_research.defect_breaker`, template hash `agent-014-6.6`, the last run of the streak as evidence — because the broken tool did not produce the claim (`an_opened_breaker_files_a_valid_platform_defect_entry_naming_the_tool`; the `UNWIRED` row is deleted and `.entry_claim(` is on the must-be-called list).

**What is not proved.** The drain/resume/wait path is reached by production code and its decisions (`plan`, `drained_unresumed`, `pause_line`, `defect_entry`) are unit-tested, but **no test drives a real child through a drain and a resume**: that needs a live `AgentManager` and a provider, which this suite does not have. The paths that touch the database — `session_leases`, the notes, the adoption — are exercised only through `pg_session_leases.rs` and by reading. A mutation check (e.g. removing the `resume_drained` call) is caught by `the_session_driver_calls_the_modules_it_was_given`, not by behaviour.

**Final marker `PARTIAL`.** Acceptance 1 (a lease never exceeds the remaining day budget) holds in `execute::lease`'s tests but **not in production**: the day budget is in dollars, a run is metered in trials, and `day_bound` answers `NotDenominated`, so the day cap never binds. Acceptance 2 (three concurrent children cannot together exceed it) is a sum in the schema and the lease request, and the Postgres test proves the sum — but the driver starts one child per iteration, so no three children ever run together. Acceptances 3 (the report turn, now via the cap), 4 (proposals never auto-approved — a type) and 5 (the breaker pauses naming the tool, and now files the entry) are met.


**2026-10-04 (F-SESSION) — the day is in trials and the driver starts up to N children together. ADR-P11-05.** (1) `session_day_budget` (0161) holds `trials_per_day`; the create route fills `DEFAULT_TRIALS_PER_DAY = 3 x DEFAULT_TRIALS_PER_TASK = 120` (editable: `PUT /api/research/sessions/{id}/day-budget`, `POST` body `trials_per_day`), `execute::day_bound(budget, unit, trials_per_day)` turns it into a cap in the unit a run is metered in, and `SessionWorker::day_cap_for` reads it (a failed read is `Unknown`, a session predating 0161 is `NotDenominated` and says so on every lease). `a_day_in_trials_caps_a_lease_and_a_spent_day_refuses_one` and `a_day_in_trials_is_a_cap_only_when_somebody_gave_one_...` (mutated: both fail). *Rejected:* a new field on `SessionBudget` (struct literals in another track's route) and converting `usd_per_day` (no per-run price exists). (2) `iterate` now starts up to `slots` children (`start_one` x N, each re-folding the log and sizing its lease against the leases already committed), awaits them concurrently (`await_all`, `tokio::join!` over 1-3; `futures` is not a dependency) and settles them one at a time (`settle_children`/`settle_ended`: the harvest, stall check and digest all write one log). **Not proved end to end:** the test that would show three overlapping children and `sum(leases) <= day` is `three_children_run_together_and_their_leases_never_exceed_the_day_in_trials`, and it cannot pass until `send_turn` can start a Research run (S-11.1). The day-gate and lease arithmetic is unit-tested; the overlap is not demonstrated.


**2026-10-04, after the manager fix — `DONE`.** The real-turn test drives a continuous session with three seeds, 3 slots and a 90-trial day through the real `AgentManager`: the fake provider sees at least two model calls in flight at once (children genuinely overlap), every lease is in trials, and at the instant each lease was issued everything then committed fits the day (a property of the `session_leases` rows, not of timing). Mutation (`wanted = 1`) fails it with "no two leases were ever committed together". Acceptance 3 (report turn on lease exhaustion) rests on the cap reaching `send_turn`, unchanged and unit-tested; 4 and 5 were met. **A second defect surfaced and is fixed:** a fresh session's first iteration read an empty board, ingested its seeds and then decided `on_drain` over the board it had read before them, so a new continuous session **completed at once** with three queued seeds. `ingest_seeds`' count now triggers the re-read. `crates/api/tests/a_session_runs_real_turns.rs` (2 tests, both pass, against the scratch database with a fake Anthropic server): `a_bounded_research_run_is_a_session_with_one_item_through_a_real_turn`, `three_children_run_together_and_their_leases_never_exceed_the_day_in_trials`.

### S-11.6 Schedule windows and the day budget — M
`DONE`

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

**2026-10-03 — re-checked; no change in kind.** The window gate, the `drains_at` boundary on `run_started`, and the day-budget gate are all in the loop (`execute::plan`, `the_gate_closes_outside_the_window_and_names_when_it_re_arms`). The gate now reads committed leases as well as spend. The two missing halves are unchanged and both are outside this crate: suspending a live child at the boundary needs AGENT-010 §10.1's per-run suspend (no status word or signal exists, so `Step::Drain` is recorded as unperformable rather than faked), and a day budget in **trials** does not exist — `usd_per_day` is the only per-day figure, so `day_bound` answers `NotDenominated` and every lease names that gap in `unbounded_by`. The fourth clause (a charter without a day budget carries the policy default) holds (`a_day_budget_is_pre_filled_rather_than_required`).

**2026-10-03, wave 2 — the drain is performed.** See S-11.5's wave-2 note for the mechanism. Against the acceptance clauses: *closing the window mid-step suspends at the boundary with no lost effect and zero tokens until the next window* — performed (`await_or_drain` → `drain_run`; the child holds no process and spends nothing while drained; the job sleeps, polling no model) but unproved end to end; *the next window resumes before claiming* — performed (`Step::Resume` is handled before any claim, `plan`'s own ordering, `every_plan_ends_in_exactly_one_claim_or_one_hold`); *a day budget hit behaves identically and resets at the window* — **not true in production**, because the day budget is in dollars and runs are metered in trials, so the gate cannot fire (`DayBudgetExhausted` is handled and waits for `next_open_on_a_later_day`, and is exercised only in `execute`'s tests); *a charter without a day budget carries the policy default* — true. **Final marker `PARTIAL`**, for the day-budget clause and for the absence of an end-to-end drain test.


**2026-10-04 (F-SESSION) — the day budget now binds.** The clause that was false in production — *a day budget hit behaves identically and resets at the window* — is now true by construction: the gate reads a day in trials (S-11.5), and "today" begins at the **schedule's own local midnight** (`OpenHours::day_start`, `a day the budget resets on is the schedule's own local day`; mixed or no zones are the UTC day and say `"UTC"`), not the database server's. Still `PARTIAL` for the drain: no test drives a real child through a drain and a resume (needs the S-11.1 manager fix).


**2026-10-04, after the manager fix — `DONE`.** The day budget binds in trials and resets at the schedule's local midnight (above); the gate and lease are exercised through real children in `three_children_run_together_...` (the leases were sized against the day). The drain/resume boundary is performed by `await_or_drain` / `resume_drained` with the monitor wake, and its decisions are unit-tested; I did not write a test that closes a window mid-step on a live child, so that sub-clause is evidenced by reading and unit tests, not by a live drain. Marked `DONE` on the four acceptance clauses as stated (suspend at boundary - performed; resume before claiming - `plan` ordering test; day budget identical and resets - proven; default day budget - proven). If a reviewer wants the live drain, it is one more test on the same harness.

### S-11.7 The couplings: kill switch, tier, platform defects — M
`DONE`
`couplings.rs` is 737 lines with 20 tests: `Observed<T>`, `KillSwitch`,
`ProfileHealth`, `DefectBreaker` (§6.6 — the rule `harvest` structurally cannot
decide), `evaluate`, and `Verdict::CannotDecide`.

One defect found and fixed in it: `Observed<T>` was **internally tagged**, so
`Observed::Is(KillSwitch::Armed)` serialised the measurement as a *key* and an
`Observed<f64>` would have failed at runtime with nothing failing at compile
time.

**Re-checked 2026-10-03: the "no production caller" sentence was stale.** `couplings::evaluate` is the first call of every iteration (`SessionWorker::iterate`), the kill switch is read from `monitor.kill_switch`, the tier through `health_of`, and the defect breaker through `child_end`; an unread switch is `CannotDecide` and holds without pausing (`a_failed_read_holds_the_iteration_without_pausing_the_session`, `an_unread_kill_switch_reaches_the_module_as_neither_armed_nor_tripped`, `two_defects_on_one_tool_open_the_breaker_and_an_unattributed_failure_does_not`). `deploy_paper` is refused while tripped in `routes::proposals` (one definition; `couplings::refuses_ring` deliberately uncalled). **Still `PARTIAL`:** "children suspend at the boundary" needs AGENT-010 §10.1's per-run suspend, which the run loop does not have (the driver records the unperformable `Drain` step instead); and §7 row 10's `platform_defect` memory entry is not written (`DefectBreaker::entry_claim` needs a platform provenance in `memory_store`, another track).


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

**2026-10-03, wave 2.** The kill switch now **drains** live children (`await_or_drain`, and `perform_drains` for children nothing is awaiting), the pause is appended, and when the switch is reset the pause is **lifted automatically** and the drained children resume before anything new is claimed; the digest carries the trip, the drain, the reset and the resume. The breaker files its `platform_defect` memory entry (`defect_entry`). Against the acceptance clauses: *tripping pauses every continuous session within one monitor event and `deploy_paper` is refused while paused* — the refusal is `routes::proposals`'; the pause is **polled** (5 s while a child runs, 30 s while waiting), not pushed by the event, so "within one event" is within one poll; *reset resumes* — yes; *withdrawing a scorecard pauses a session on that profile* — **not met**: `health_of` reads the profile's static `tier`, and nothing records a withdrawn or degraded scorecard to read; *the digest carries each transition* — yes. **Final marker `PARTIAL`**, for the withdrawn-scorecard clause and the push-versus-poll gap.


**2026-10-04 (F-SESSION) — both open clauses closed: `DONE`.** *Withdrawn/degraded scorecards are recorded:* `profile_standing` and `profile_judgement` (0163, append-only, `app_role` INSERT only), written by `POST /api/research/profiles/{profile}/scorecard` (human-scoped) and read by `SessionWorker::profile_health` for a continuous session; a hosted provider whose credential has been **removed** is `withdrawn` whatever the last scorecard said (`session_profile::credential_withdrawn`), a recorded standing cannot lift the static frontier rule, and the pause event carries `tier_detail` (who said so). Proven: `a_recorded_scorecard_is_read_back_and_a_later_degraded_one_wins` (Postgres, `app_role`), `a_degraded_or_withdrawn_standing_pauses_...`. *The pause is event-driven:* `session_wake::Wake` LISTENs on the monitor's own channels (`monitor_kill_switch`, `monitor_events`) and `await_or_drain` / the wait sleep wake on them; the 5 s / 30 s poll remains only as the backstop (`Woke::Timer`). `a_monitor_notification_wakes_a_session_within_one_event_and_the_heartbeat_does_not` (Postgres: a kill-switch NOTIFY ends a 30 s sleep in well under 5 s; a `monitor_lease` heartbeat does not), `every_channel_the_monitor_publishes_is_classified_here`. `deploy_paper` refusal while tripped is unchanged (`routes::proposals`). ADR-P11-04 stands.

### S-11.8 The stall detector — M
`DONE`

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

**2026-10-03 — `DONE`.** Re-checked against the four clauses with the caller in place: `SessionWorker::check_stall` runs `stall::detect` after every finished run over a five-way join (`finished_runs`) and appends `session_stalled` with the diagnosis and a `not_measured` block; six barren runs trip `barren_streak` naming the six (`stall.rs` and the driver's `a_barren_stretch_of_rows_reaches_the_detector_and_trips_it`); refinement-only trips `degenerating`; a stalled session fails `Folded::can_claim` so `iterate` returns before anything is claimed (`a_session_that_is_not_running_claims_nothing`); and every fixture goes through `detect_checked`, so no suggestion touches `delta_practical`. The stall also forces a digest (S-11.12). Q-S11-02's cost-drift reading was settled by ADR-D-07 in AGENT-014 §7.1.

### S-11.9 Autonomy, the frontier-only rule, and the `judgement` threshold — S
`DONE`
The crate half is `charter.rs` (1,063 lines, 21 tests): `Judgement`,
`Profile`, `admit`, `TierRefusal`, and `AlwaysAsks::asks`, which takes autonomy
and ignores it on purpose.

**`charter::StopWhen` had no reader anywhere in the repository** — five words, a
`Serialize`, and nothing that evaluated one. It has one now, and
`ScheduleEnd` is reported **not evaluable** with its reason rather than silently
never firing: `Schedule` holds weekly windows and no end date, so a charter
listing it believes the session will stop and it will not.

**2026-10-03: `charter::admit` has its caller.** `routes::research_sessions::create` calls it through `admit_at_creation` with a `charter::Profile` built from the loaded harness profile (`charter_profile`). A non-frontier profile is refused with the profile named; `autonomous` is refused, not ignored, with `admit`'s own sentence (the hard-coded "nothing reads that threshold yet" refusal is gone); a new session defaults to `notify` (`a_new_session_defaults_to_notify`). `AlwaysAsks` is unchanged and still asks under every autonomy (`autonomy_is_a_setting_on_starting_only`). `StopWhen::ScheduleEnd` is now `{ at: Option<DateTime> }` and **fires** at its date (`a_schedule_end_with_a_date_fires_at_that_date_and_not_before`); an undated one stays not-evaluable and says so; old `{"kind":"schedule_end"}` rows still read (`a_schedule_end_written_before_it_carried_a_date_still_reads`). The driver passes `Stopping::now`.

**ADR-P11-04 (2026-10-03) — an unmeasured `judgement` refuses `autonomous` and is a note under `notify`.** Nothing on this platform records a `judgement` pass rate per (profile, workflow): `eval_scorecards.metrics` is a harness-version aggregate. So `charter_profile` declares `measured: false` and an empty map rather than guessing. Strictly, `admit` then refuses every charter. *Rejected:* refusing continuous research outright until the suite runs (it would make the toggle inert on every deployment until an eval track lands) and *rejected:* treating unmeasured as passed (it is the one reading §8 forbids). Taken: under `autonomous` the refusal stands; under `notify`/`supervised` a refusal that is **only** about the missing measurement admits the charter and writes the refusal's sentence on the session's log as a `note` (`judgement_not_measured`), because a human is told about every start — the property the threshold protects. A refusal about what the profile *is* (not frontier, a measured failure) is never softened. **`PARTIAL`** until a per-(profile, workflow) `judgement` store exists (S-10.16/eval track); the `charter::admit` row in the driver test's `UNWIRED` list says exactly that.


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

**Final marker `PARTIAL`** (wave 2 re-check). Three of the four acceptance clauses are met with a production caller: a non-frontier profile is refused with the profile named, `autonomous` is refused (not ignored) — today for every profile, because no `judgement` pass rate is recorded — and a new session defaults to `notify`. The fourth, *under `autonomous` a `Promotion` action still suspends for a human (adversarial)*, is held by `charter::AlwaysAsks` and its sweep over every autonomy, but `AlwaysAsks::asks` has no caller in the loop: a proposal reaches *Needs you* as a type with no decision field, which is the same outcome by another route, and nothing maps a `harvest::RunProposal` to one of the three words. And `autonomous` can never be *granted* until the `judgement` store exists, so the half of the task that matters when it is on is untested in production.


**2026-10-04 (F-SESSION) — the `judgement` store and the loop's caller exist: `DONE`.** `profile_judgement` (0163) holds the pass rate per (profile, workflow) with the line it was judged against; `session_profile::charter_profile` builds §8's `Profile` from it and from the recorded standing (`measured` only when the latest standing says so; the profile's own `autonomous_threshold`, strictest 1.0 when undeclared), and the create route admits through it. `autonomous_is_granted_on_a_recorded_passing_judgement_and_refused_below_the_line` is the first time `autonomous` can be granted, and only on a measurement. `AlwaysAsks::for_proposal_kind` maps a harvested proposal to the standing rule (`promote_strategy`/`change_limits` -> `Promotion`, `deploy_paper` -> `SpendAboveAskLine`) and the loop consults `.asks(autonomy)` for every proposal it routes to *Needs you* (`standing_rule` is on the `awaiting_approval` event); both rows left `UNWIRED` in `the_session_driver_calls_the_modules_it_was_given.rs` are closed and the rest of that list re-stated. Mutation: dropping the `deploy_paper` mapping fails `a_proposal_kind_maps_to_the_standing_rule...`. **Remaining, named:** nothing *runs* the `judgement` suite and posts its scorecard, so until an eval track does, every profile is unmeasured and `autonomous` is refused (ADR-P11-04's note under `notify` stands); the route is the producer's door.

## Sources that watch the world, and the endless setting

### S-11.10 Observation subscriptions: regime, coverage, monitor — M
`DONE`
`sources/observe.rs` (474 lines, 11 tests) carries the regime, coverage and
monitor subscriptions, rate-limited on a fingerprint held in `source_refs`
rather than in a fourth table.

**Re-checked 2026-10-03: the "no production caller" sentence was stale.** `sources::ingest_for` runs every iteration and `SessionWorker::world` feeds §4.7 from `monitor.events` (divergence / scaled-down) with the rate limit on `source_refs`. Clauses 2 and 3 (an in-scope `scaled_down` inserts a `diagnose` citing the event; out of scope inserts nothing) are reached in production and tested in `sources/observe.rs`. **Clause 1 is `BLOCKED:UPSTREAM`:** `Observation::RegimeFlip` has no producer — `JobKind::InstrumentProfile` exists in `crates/jobs` and no worker claims it (S-4.7's profiler is itself built-but-unreached), and coverage extension lives in ClickHouse behind no pool the driver holds. Both arrive empty and are named on the ingest note.


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

**Final marker `PARTIAL`, with clause 1 `BLOCKED:UPSTREAM`** (wave 2 re-check): the regime-flip producer is S-4.7's profiler, which has no worker; clauses 2 and 3 are met in production.


**2026-10-04 (F-SESSION) — clause 1 unblocked and wired: `DONE`.** `JobKind::InstrumentProfile` has a worker, and `regime_causal.regime_state` records a statistical label per scope per day, so a regime flip has a producer. `session_regime::flips` reads label changes (partitioned by `model_version`, so a refit is not a flip; `since` is the day the label changed; the subject is the scope's `series_symbol`, falling back to the scope name) and `SessionWorker::world` feeds them to `sources::ingest_for` beside the monitor events; `Raised` (derived from `source_refs`) holds the second and third reads out. `a_regime_flip_has_one_identity_across_three_reads` (Postgres, `app_role`: three reads of one `low_vol -> high_vol` flip are one observation with one fingerprint). Coverage extension is still not an event (nothing publishes a crossing) and is named on the `subscriptions_not_measured` note.

### S-11.11 Replenish — the endless setting — M
`DONE`
`replenish.rs` (614 lines, 18 tests): `Drain{NotDrained,Waiting,Replenish,Complete}`,
`on_drain`, `absorb`, and a fruitless survey handed to `stall` as
`knowledge_units = 0` rather than as an absence the stall detector would
mistake for silence.

**2026-10-03: the switch is on.** `replenish::on_drain` has been called by the driver since 42739eca, but `routes::research_sessions::create` still **refused `replenish = true`** ("S-11.11 is not built"), so no session could ever reach it. The refusal is removed; the flag is stored and echoed in the create response. A drained agenda under `replenish = true` now writes a `survey` item; under `false` the session completes (`a_drained_agenda_ends_the_session_only_when_it_is_not_set_to_keep_going`). **Still `PARTIAL`:** `replenish::absorb` has no caller — nothing stores a survey's `proposed_questions[]` or their value estimates, so a survey's questions reach the agenda through the ordinary harvest (as `open_question` entries, scored like any other source 4.3 item) without §4.9's estimates, and a fully-deduped survey reaches the barren streak through its own yield rather than through `absorb`'s explicit zero.


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

**Final marker `PARTIAL`** (wave 2 re-check): clauses 1 and 2 are met in production (a drained agenda under `replenish = true` writes a `survey` item; under `false` the session completes). Clause 3 — *a fully-deduped replenish increments the barren streak* — rides on the survey run's own yield, not on `absorb`'s explicit zero, because nothing stores a survey's proposals.


**2026-10-04 (F-SESSION) — `replenish::absorb` has its caller: `DONE`. ADR-P11-06.** A `survey` cannot spend a trial, so its "what to ask next" is the `open_question` entries it files; `session_survey` reads them back by run, stores them with their estimates (`session_survey_proposals`, 0162) and `SessionWorker::absorb_survey` hands the report to `absorb` before the harvest's offers are planned (a survey's open questions are no longer also offered as source 4.3 items, which would double them). The absorption (`session_survey_absorptions`, written once) holds the **explicit** count of genuinely new questions; `finished_run` reads it as the survey's yield and treats the survey's missing `trials_spent` as the fact it is (zero cost), so a fully-deduped replenish counts toward the barren streak (`an_absorbed_survey_is_never_left_out_for_having_no_trials_and_its_yield_is_the_explicit_count`; `a_run_that_cost_nothing_has_no_cost_per_unit...` keeps free surveys from dragging the cost-drift median to zero). The estimates ride on the `survey_absorbed` note per item. *Estimates are declared, not measured:* `curious 0.25 / useful 0.5 / decisive 1.0`, stored with their label, NULL (never 0) when the run gave none. *Rejected:* a `value_estimate` field on `AgendaItem` (struct literals in `sessions.rs`; a `Score` from one number would put unmeasured signals on a decision row). Proven against Postgres: `a_surveys_proposals_are_stored_with_their_estimates_and_a_deduped_survey_is_an_explicit_zero`.

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

**2026-10-03 — re-checked: the cadence and the triggers are wired; the card is not.** `maybe_digest` writes on `review_every` (run count over the period, or the window boundary), `force_digest` on stall, complete and stop, persisted to `session_digests` with `spend_unit`, and `digest_written` on the log. New: the digest carries P3's dedupe scorecard (S-11.4). Clauses 1–3 hold at the object and driver level (`digest.rs` 16 tests incl. `unresolvable()` asserted empty; `the_period_is_everything_since_the_last_digest_and_nothing_before_it`). **`PARTIAL`** because the task's file list includes `frontend/src/components/desk/cards/Digest.tsx`, and "every line resolves on click" is a rendering claim that belongs to S-11.13's track.

**Final marker `PARTIAL`** (wave 2 re-check): the object, cadence, triggers, persistence and the resume/drain lines are done and tested; the card and the click-through are `Digest.tsx`, S-11.13's track.


**2026-10-04 (F-SESSION) — content verified, one gap closed.** Re-read against §2.4's list: verdicts (no titles), findings established/refuted, negatives with new classes, questions opened/closed, proposals *still awaiting*, budget with the unit in the sentence, the next five with scores and why, every pause with its reason (now including `tier_detail`), the dedupe scorecard, and — new — **contradictions found/resolved in the period** from the pair store (it was always empty: a pair belongs to memory, not a run). `Digest::unresolvable()` is asserted empty on the new lines. `PARTIAL` only because `Digest.tsx` and the click-through are the desk track's.

### S-11.13 The desk: the toggle, the charter form, the Agenda and Charter tabs, the rail, home — L
`PARTIAL` 2026-10-04 (track DESK) — **checked each clause against the tree; the rail's server half was the one real gap and is closed.** The toggle, charter form (with the schedule grid), Agenda and Charter tabs, journal slash commands, header badge/budget and the Desk-home rows were already built and tested (`a-session-is-one-row-not-a-page.test.tsx`, `the-journal-hears-its-session.test.tsx`, `the-day-spend-is-what-the-digests-say.test.tsx`); the grouping lib (`lib/desk/rail.ts`, `DeskPage`) already preferred `parent_conversation_id` but **`GET /api/agent/conversations` never selected it**, so the rail could only group through agendas. `conversations::list` / `get` now select `agent_conversations.parent_conversation_id` (migration 0138's column) and send it on every row (`ConversationSummary.parent_conversation_id`), so a child groups under its journal even after its agenda item is pruned. Frontend suite: 735 passing across `components/desk`, `lib/desk`, `pages`, `api`. **Still missing:** a veto preventing its claim and showing in the digest is the session driver's and digest writer's (S-11.5 / S-11.12); a live spend-against-day meter needs a spend-per-day route (the bar is still the digest floor); the parent-column change (`conversations.rs`) compiles (api lib tests build) but its SQL is not exercised against Postgres; 375 px and both themes held statically.

Earlier note: `PARTIAL` 2026-10-04 (track F-UI) — **the day-budget bar now shows spend, from the one source that records it.** No route counts dollars against a day, but each digest carries `budget.spent` and a `spend_unit`: `daySpendFromDigests` (`lib/desk/sessions.ts`) sums today's (UTC) digests whose unit is `usd`, and is `null` — "not recorded", never zero — for trials, an unrecorded unit, another day's digest or no digest; the journal header draws `$3.50 / $25.00/day · from digests` (a floor, not a meter: spend since the last digest is not in it) (`the-day-spend-is-what-the-digests-say.test.tsx`, 3 cases; desk suite 676 passing). The Desk-home row still says spend is not recorded (it would need a digest read per session). **Still missing:** "a veto prevents its claim and shows in the digest" is the driver's and the digest writer's (S-11.5/S-11.12); the digest card (`DigestCard.tsx`) already renders whatever line the writer emits and the budget line from `spend_unit`; a live meter needs a spend-per-day route. 2026-10-03 note follows. — **the desk half is built on the session API as served; what is left is on the server side or needs a browser.** Composer: Chat / Research with the Continuous toggle; the charter form gained the **7 × 24 schedule grid** (Monday first, empty = any time, first click materialises the full week — the shape `desk_charter::windows_from_grid` refuses otherwise), and draws `autonomous` and *keep generating questions* disabled with the reason, because the create route refuses both today (S-11.9, S-11.11). Toggling keeps typed constraints both ways. Journal: `/agenda add …`, `/pause`, `/resume`, `/charter` with handlers (`slashAction`); the journal **subscribes to `…/sessions/{id}/events`** and re-reads board, charter and digests on each frame (the desk half of S-11.14's "within one event"). Agenda tab: **Add a question** reaches `POST …/agenda` (it was a button with no handler), running cards show the child's stage strip from `/api/desk/running` or say it is not reported, veto/approve/reorder as before. Charter tab: an **Amend** form that sends the stored charter back through `PUT …/charter` with only the amended fields changed. Header: *continuous* badge, agenda depth, day budget. Desk home: one row per continuous session in *Running now* (state, next item and how the board knows it, agenda depth, day budget), a stalled one in *Needs you* with its diagnosis and **resume · amend · stop** (amend opens the journal on `?tab=charter`). No user-visible "Program" (`no-deprecated-aliases.test.ts`). Tests: `a-session-is-one-row-not-a-page.test.tsx` (25), `the-journal-hears-its-session.test.tsx` (2). **Missing:** spend against the day budget — no served route counts dollars against a day, so the bar states the budget and says spend is not recorded; "a veto prevents its claim and shows in the digest" is the driver's and the digest writer's (S-11.5/S-11.12 track) — the desk posts the veto and renders whatever digest is written; 375px and both themes are held statically (`the-desk-varies-only-by-tokens.test.ts`), not in a browser.

Earlier note: NOT STARTED 

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

**2026-10-03 — one more route, no change to the three open gaps.** `POST …/{id}/agenda/{item}/redirect` (`redirect_item`, §6.4) is written, asks `web:full` like every other mutating handler (added to `every_mutating_handler_asks_the_same_question`), and refuses a non-live item; it is **not registered** — `routes/mod.rs` is the orchestrator's. `create` now reports `replenish`, `trials_total`, `judgement` and `removed_workflows`. The three gaps above stand: the agent-token refusal is a source scan, not an HTTP test; `lifecycle` is at 9; the SSE half waits for S-11.13.

**Final marker `PARTIAL`** (wave 2 re-check): unchanged — the agent-token adversarial test is a source scan, `lifecycle` has 9 tools and not 12, and the SSE clause needs a journal; the redirect route still needs its `routes/mod.rs` line.


**2026-10-04 (F-SESSION) — `lifecycle` is at 12; the SSE is pushed. ADR-P11-08.** Three more read-only `.internal()` tools in a new file (`tools/lifecycle_sessions.rs`): `get_stage` (backed by new `GET /api/research/sessions/current/stage`, keyed on the caller's own run), `get_charter`, `list_contradictions` (backed by the pair store). 9 -> 12; `no_namespace_holds_more_than_sixteen_tools` and the descriptor budget stay green; additive registration only (`tools/mod.rs`, `spec.rs`, `lib.rs`). *Rejected:* `refine`, `compare_candidates` and `reproduce`, which AGENT-013 section 8 names: each needs a platform backend (refinement chain, the comparison protocol over real measurements, a receipt re-run) that is another task's, and a tool with no backend is the defect this repository keeps finding. **SSE:** `GET /api/research/sessions/{id}/events` (served since 2026-09-21, `Last-Event-ID` resumes by `seq`) now wakes on a Postgres NOTIFY (`session_events`, trigger in 0164) instead of a 2 s poll, so an agenda transition reaches an open journal within one event (`every_session_event_publishes_its_session_id_for_the_stream_to_wake_on`, Postgres, `app_role`); the poll is only the backstop. The redirect route and every session route are registered (`routes/mod.rs`; plus `day-budget`, `profiles/{p}/scorecard|standing`, `contradictions`, `sessions/current/stage`). **Still `PARTIAL`:** the agent-token adversarial clause is still a source scan over the handlers (a real service-token HTTP test needs an `AppState` and a minted token); the desk client for the stream is another track's.

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
