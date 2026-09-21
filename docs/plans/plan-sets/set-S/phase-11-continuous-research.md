# Phase 11 — Continuous research: Research with the toggle on

**Completion: 14% (0 done / 2 partial of 14 tasks)** — S-11.1 (the object, the fold, the driver) and S-11.14 (the API and the three read tools) landed 2026-09-21; each names what it does not cover.

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
`NOT STARTED`

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

### S-11.4 Duplicate work is refused three times — S
`NOT STARTED`

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

## Execution

### S-11.5 Execution: a conversation per item, the brief, the lease, the harvest, failure — L
`NOT STARTED`

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

### S-11.6 Schedule windows and the day budget — M
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

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
