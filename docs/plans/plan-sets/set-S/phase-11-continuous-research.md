# Phase 11 — Continuous research: Research with the toggle on

**Completion: 0% (0 / 14 tasks)**

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
`NOT STARTED`

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
`NOT STARTED`

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
