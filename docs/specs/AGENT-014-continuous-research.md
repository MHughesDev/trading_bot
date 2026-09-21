# AGENT-014: Continuous Research — Research with the continuous toggle on: a charter, an agenda, and an agent that chooses its own next question

> **Status:** **Partially implemented** — Set S phase 11. Re-checked 2026-09-21.
>
> §2, §3, §5 and §10 are built: the session object, the append-only fold, the scored agenda with its forced exploration share, the API and three read tools. Leases, the harvest, the stall detector, schedule windows, the digest and sources 4.3–4.9 are not.
>
> **The live count is in [`docs/plans/plan-sets/set-S/phase-11-continuous-research.md`](../plans/plan-sets/set-S/phase-11-continuous-research.md), and the programme total is in [`docs/plans/PLAN.md`](../plans/PLAN.md).** It is deliberately not restated here: a number written into a spec is a number nobody recomputes, which is how this line came to say "ready to build" about code that was already serving traffic. `crates/invariants/tests/the_plan_counts_what_the_plans_say.rs` holds those two files honest; nothing holds a hand-copied number here.
**Date:** 2026-09-14 (revised 2026-09-15: continuous research is a toggle on the Research mode, not a third mode — user decision, ADR-0037 §2)
**Author:** Claude (Fable 5.1), as architect and product designer
**New:** the "campaigns, regime engine, autonomy" remainder of reserved Set R-b (`17_BUILD_ORDER §2`, Set S MASTER §2) — specified here as **continuous research sessions**, distinct from and built beside the platform's `mlops.campaign`
**Depends on:** AGENT-013 (modes, lifecycle, yield, proposals, open questions), AGENT-010 (runs, effect log, budget leases, sub-runs), AGENT-007 (memory as the agenda's largest source), AGENT-009 (the `judgement` suite), AGENT-012 (the desk), COMP-007 (monitor events, kill switch)
**Related:** ADR-0037, ADR-P2-04 (event-sourced fold on the job service — the pattern this reuses), SPEC §4.4 (decision log), INV-21 (exploration floor)

---

## 0. Why this spec exists

The North Star asks for an agent that supports "both focused assignments and continuous research across long-running sessions", that "autonomously generates hypotheses … and chooses productive next steps", and whose continuous operation handles "task selection, checkpoints, recovery, resource budgets, duplicate-work prevention, and pause or stop conditions". None of that is a property of a run. A run has one goal and is designed to end (AGENT-008 §4, AGENT-010 §6). What continues across a long session is the **choosing** — and choosing needs state of its own, a budget of its own, failure modes of its own and a supervision surface of its own.

The user sees two modes: **Chat** and **Research**. Research carries a **Continuous** toggle. With it off, Research is one question, one run, one verdict (AGENT-013 §5). With it on, the question field becomes a **charter**, the run becomes an **agenda** of runs, and the session acquires everything this spec describes. The toggle is a *qualifier*: it changes what the user hands the agent and what the agent hands back, not what the agent may spend or touch (the rings are identical — AGENT-013 §4). Internally the two are one object, `ResearchSession { continuous }`; a bounded session is the case with exactly one agenda item, which is what makes the merge honest rather than cosmetic.

Two things already exist and neither is this object:

- `mlops.campaign` (SPEC §10; built per IMPLEMENTATION-CHECKLIST 2.1) is an *optimisation* over one hypothesis family under one DEFINE. It answers "search this space well". It cannot decide that the space is the wrong one to search.
- FEAT-003 §8's campaign loop (islands, bandit, moves, curate) is the agent's strategy *inside* HYPOTHESIZE. It is one family deep.

A continuous session sits above both: it decides *which question is worth a run*, hands each to a Research run, reads what came back as knowledge (yield, open questions, negatives, contradictions), and decides again. "Endless" is the session's `replenish` setting — it regenerates the agenda when it drains.

## 1. Principles

- **P1 — A session is a fold, not a process.** Its state is an append-only `session_event` log driven by a job (ADR-P2-04's pattern); every child run is idempotent on `(session_id, agenda_item_id)`; recovery is replay plus AGENT-010 §5 resume of the live child. There is no status column and nothing else to recover. A bounded session is the same fold with one item.
- **P2 — The agenda is scored, not vibed.** The next item is chosen by a declared policy over declared signals with a forced exploration share; the choice is a `decision` row with its full candidate set and propensity (SPEC §4.4), so the policy can be audited and, later, learned.
- **P3 — Duplicate work is refused three times.** At agenda insert (dedupe key), at intake (`hypothesis_precheck`), and at the ledger (the family's trial count). A session that re-tests a dead idea has failed at all three, and the scorecard says which.
- **P4 — A session spends budget through leases.** It holds a budget over days; each child run holds a lease split from it (AGENT-010 §9's `BudgetLease`, not `Clone`). A session cannot overspend by concurrency because a lease is spent by exactly one run.
- **P5 — A session can be told it is degenerating.** A research programme that only makes ad hoc adjustments and produces no novel results is stalled, whatever its dashboards say (Lakatos). The stall detector reads yield (AGENT-013 §10) and pauses for a human with a diagnosis rather than spending budget on motion.
- **P6 — Autonomy is a setting on starting, never on approving.** `supervised | notify | autonomous` governs whether the human confirms each agenda item *before it starts*. The hard envelopes (`SealedHoldout`, `Promotion`, `Spend` above the ask line) ask in every setting; ring 3 is unreachable in every setting (AGENT-013 §4).
- **P7 — The human's cost of returning is one digest.** A session that runs for a week is useful only if reading what it did takes minutes. The digest is a first-class object, written by the platform from yield and verdicts, not by the model's prose.

## 2. Objects

### 2.1 `ResearchSession`

```
{ id, conversation_id,                     # the conversation IS the session; continuous → it is the journal
  continuous: bool,                        # the toggle
  question?: ResearchQuestion,             # continuous = false
  charter?: Charter,                       # continuous = true
  state: draft|running|paused{why}|stalled{diagnosis}|complete|stopped,
  budget: {usd_total, usd_per_day, trials_total, max_runs, spent: {...}},   # bounded: the run's own budget
  schedule: {windows: [{days, start, end, tz}], max_concurrent_runs: 1..3}, # continuous only
  autonomy: supervised|notify|autonomous,   # continuous only; default notify
  replenish: bool,                          # continuous only; "keep generating questions" — the endless setting
  review_every: {runs: n} | {window: 1},    # continuous only; digest cadence
  tier: profile_name,                       # continuous requires a frontier profile (§8)
  created_at, last_digest_at, event_seq }
```

**Qualifiers — what the continuous toggle turns on.** Charter instead of question; `schedule`, `autonomy`, `replenish`, `review_every`, `max_concurrent_runs`; the agenda (§4–§5); leases (§6); the stall detector (§7.1); the digest (§2.4); the frontier-tier requirement (§8). Everything else — model, workflows, rings, budget, the controls row on any live child — is shared with bounded Research.

**Budget defaults.** `usd_per_day` defaults from the desk's budget policy (AGENT-012 §12; platform default **$25/day**), `usd_total` to 30 × day, `trials_total` from the tier's `max_trials_per_task` × `max_runs`, `max_runs` to 60. All pre-filled in the charter form and editable; none is required input.

### 2.2 `Charter`

What the user writes instead of a question when Continuous is on:

```
{ scope: {instruments[], asset_classes[], families[], timeframes[], windows?},
  objective: {kind: find_deployable{constraints: SPEC §10 objective shape}
                  | build_knowledge{about: [...]}
                  | maintain{strategy_ids[]}          # re-verify, watch for decay, diagnose
                  | mixed[...]},
  seeds: [ResearchQuestion-shaped items, optional],
  permitted_workflows: [...],            # default: all the tier supports (AGENT-013 §11)
  exclusions: [subjects, families, tools, data sources],
  standing_constraints: [...],           # become user_preference rows scoped to the session
  exploration_share: 0.15,               # ≥ 0.10, cannot be set below (mirrors INV-21)
  stop_when: [{budget}, {schedule_end}, {agenda_drained}, {stall}, {n_deployable_found: k}],
  delta_practical_default?: f64 }        # a *suggestion* for pre-registrations; each run still seals its own
```

A charter is versioned; editing one appends a `charter_amended` event and is shown in the digest. Its `standing_constraints` are written as `user_preference` entries with `scope = session:<id>` (AGENT-007 §3.4) so every child run's brief carries them at top rank.

### 2.3 `AgendaItem`

```
{ id, session_id, question: ResearchQuestion (AGENT-013 §7.1), workflow,
  source: charter_seed|user|open_question|retry_condition_met|contradiction|stale_finding|
          observation|refinement_followup|proposal_from_run|replenish,
  source_ref, dedupe_key, priority: computed, score: {value, novelty, feasibility, cost, total},
  status: proposed|approved|queued|claimed{run_id}|running{run_id}|done{verdict}|vetoed{by}|
          superseded{by_item}|expired,
  depends_on[], created_at, claimed_at?, finished_at?, yield_ref? }
```

A bounded session has one item, `source = user`, `status` following its single run.

### 2.4 `Digest`

Written by the platform every `review_every`: period; runs finished with verdicts (not titles); findings established and any refuted; negatives with new classes; questions opened/closed; contradictions found/resolved; proposals awaiting; budget spent and cost per knowledge unit; the next five agenda items with their scores and why; anything that paused. Each line opens the thing it names. It is the session's *Since you were away* (AGENT-012 §2 2a), scoped and scheduled, and it renders as a card in the journal.

## 3. The session loop

The driver is `JobKind::ContinuousResearch` on the job service — leased, heart-beaten, re-queued on worker loss, exactly as `JobKind::Campaign`. A bounded session does not use it (one run, started directly). One iteration:

```
fold(session_event) → state
if state ∉ {running}                       → exit (the job re-arms on the next event or window)
if outside schedule window                  → append window_closed; exit
if budget/day exhausted                     → append day_budget_exhausted; exit until window
ingest sources (§4)                          → append agenda_item_added*
score + select (§5)                          → append selected{item, candidate_set, propensity}
if autonomy = supervised and item ∉ approved → append awaiting_approval; exit
claim item; open child conversation; start Research run with a BudgetLease (§6)
                                             → append run_started{item, run_id, lease}
on Input::RunDone (from the child's Outcome) → append run_finished{item, verdict, yield_ref}
    harvest: open questions, proposals, agenda proposals from the dossier → sources
    stall check (§7) → maybe append stalled{diagnosis}
    digest check → maybe append digest_written
if agenda drained: replenish ? run a `survey` item (§4.9) : append complete
```

Each arrow is an event; each child is created through `JobStore::submit_child_once` keyed on `(session_id, agenda_item_id)` so a driver killed between "start run" and "record started" re-derives the same child rather than starting a second (AT-59's guarantee, reused verbatim). `max_concurrent_runs` > 1 means the loop may claim more than one item per iteration; each child is its own conversation (AGENT-010 §4.1: one live turn per conversation), so concurrency never shares a workspace.

## 4. Agenda sources

The agenda is fed from nine sources. Each produces `AgendaItem`s with a `dedupe_key`; an insert whose key matches an item in any non-terminal status is dropped and the existing item's `source` list is extended — a question wanted by three sources outranks one wanted by one.

| # | Source | Trigger | Default workflow |
|---|---|---|---|
| 4.1 | Charter seeds | at start | as written |
| 4.2 | User | any time, from the agenda tab or `/agenda add` in the journal | as written; **top priority, bypasses scoring** |
| 4.3 | Open questions (AGENT-013 §9) | in scope, `status = open`, `blocking` first | `investigate` |
| 4.4 | Retry condition met | a `negative_result` in scope whose `retry_condition` now evaluates true (a capability, a data window, an adapter) | `investigate`, as variant |
| 4.5 | Contradiction | an AGENT-007 §2.2.1 pair in scope with no resolving finding | `diagnose` — resolution is a research act |
| 4.6 | Stale finding | an `established` finding in scope past `valid_until` | `replicate` (the re-verification AGENT-007 P7 promises, now scheduled by the session that cares) |
| 4.7 | Observation | instrument profile regime flip (AGENT-007 §3.3); new data coverage in scope; monitor `divergence` or `scaled_down` event on a strategy in scope (COMP-007 §6 `monitor.events`) | `diagnose` for monitor events; `investigate` for regime/coverage |
| 4.8 | Refinement follow-up | a finished run's `next_moves` / secondary hypotheses marked worth pursuing | `investigate` |
| 4.9 | Replenish | agenda drained and `replenish = true`: a `survey` run over the charter and everything in scope, whose `SurveyReport.proposed_questions[]` become items with the survey's value estimates | per proposal |

Sources 4.3–4.7 are *subscriptions*: the driver evaluates them on every iteration from memory, the ledger, the profiler and the monitor's event table. They are what makes the session react to the market rather than to its own plan — the North Star's "market observations into experiments". Source 4.7 is rate-limited per subject (one item per regime flip, not one per profiler run).

## 5. Selection

### 5.1 The score

```
total = value × novelty × feasibility / cost
```

| Signal | From | Range |
|---|---|---|
| `value` | charter objective weight for the item's subject/family × source weight (`user` ∞, `contradiction` 1.5, `retry_condition_met` 1.3, `open_question{blocking}` 1.3, `observation` 1.2, `stale_finding` 1.0, `charter_seed` 1.0, `refinement_followup` 0.8, `replenish` as estimated) | (0, ∞) |
| `novelty` | `hypothesis_precheck` at insert: `proceed_new` 1.0, `proceed_as_variant` 0.6, `reject_duplicate` **0 (dropped)**; reduced by the family's trial count in the ledger (`1 / (1 + log(1 + n_family/20))`) | (0, 1] |
| `feasibility` | data coverage for the window (from the instrument profile); every tool the workflow needs is in the tier's catalogue slice and the workflow is in `permitted_workflows`; `min_track_record` satisfiable; `get_self_status` reports no open breaker on a needed tool | {0, 0.5, 1} |
| `cost` | `estimate_cost` for the workflow's expected trials on this tier (AGENT-006 §2.4b), in USD, floored at $0.10 | (0, ∞) |

Every signal is percentile-normalised within the current agenda before combining (AGENT-005 §24.3's lesson: unbounded signals eat bounded ones). The weights are a versioned `SelectionPolicy` object with a conformance reader per field.

### 5.2 Exploration

With probability `exploration_share` (charter; ≥ 0.10) the selection is **uniform over items with `feasibility > 0` and `novelty > 0` outside the top-k** — the item the scorer would not have picked. In plain terms: at least one pick in ten is a deliberate detour, so the ranking can be shown wrong and the session can find what the charter did not anticipate. This is INV-21 at the agenda level and for the same reason. The draw is made by the platform with a platform-held seed; the share is not in any tool schema.

### 5.3 The decision row

Every selection is a `decision` (SPEC §4.4) with `decision_kind = select`, `candidate_set` = every scored item with its signals, `chosen`, `propensity` (the item's probability under the policy including the exploration mixture), `policy_id/version`, `exploration_flag`, `rationale` (one sentence the digest shows: "highest value: resolves contradiction f_31a/f_47c; novelty 1.0; est. $2.10"). This is what makes "why did it do *that* next?" answerable, and what makes the selection policy a learning-to-rank dataset later rather than a log.

### 5.4 Ties and stability

Ties break on `(created_at asc, id)`. An item's score is recomputed only when a source changes it (new corroboration, a retry condition flipping), not every iteration, so the agenda the human saw in the digest is the agenda that ran.

## 6. Execution

An approved/queued item becomes a Research run:

1. **Conversation** — one per item, titled from the question, tagged `session:<id>`, grouped under the journal conversation in the desk rail. The journal (the session's own conversation) is where the human talks *to the session*.
2. **Brief** — the child's AGENT-007 §4 brief includes the charter's standing constraints (as preferences), the session's *recent verdicts in scope* (last five, one line each, so the child knows what its siblings just found), and the item's `source` ("you are running because finding f_31a is 94 days stale").
3. **Lease** — `BudgetLease` split from the session's remaining day and total budgets, sized by the workflow's `estimate_cost` × 1.5, capped by `max_usd_per_task` of the tier. The child's `max_trials_per_task` is the lease's trial count. A child that exhausts its lease ends with the report turn (AGENT-010 §6); it may not borrow.
4. **Steering** — the human can steer, pause, approve and stop the child exactly as any Research run (AGENT-012 §3.4). A steer that redirects the goal (AGENT-008 §3.4a) also rewrites the agenda item's question, so the agenda does not lie about what ran.
5. **Harvest** — on `Outcome`, the driver reads the dossier's yield, `open_question`s, `Proposal`s and proposed next questions, feeds sources 4.3/4.8, and marks the item `done{verdict}`. Proposals into rings 1 and 2 go to *Needs you*; a session **never** approves its own proposals (P6).
6. **Failure** — a child that ends `failed`, `fenced` or `interrupted` marks the item `queued` again with `attempts += 1`; at `attempts = 2` the item is `expired{why}` and the reason is in the digest. Two consecutive children failing on the same `platform_defect` opens a session-level breaker on that tool (AGENT-008 §3.7's rule, one level up) and pauses the session: budget spent on a broken platform is not research.

## 7. Pause, stall, stop

| Condition | Effect | Resumes |
|---|---|---|
| Schedule window closed | live children finish their current step and suspend at the boundary (AGENT-010 §10.1's drain, per run); no new claims | next window, automatically |
| Day budget exhausted | as above | next day's window |
| Total budget / `max_runs` / `stop_when` met | `complete` after the final digest | — |
| Agenda drained, `replenish = false` | `complete` | user adds items → `running` |
| Human **Pause** | as window closed | human **Resume** |
| Human **Stop** | live children get the report turn; `stopped`; digest | — (a new session may be forked from its charter) |
| **Kill switch tripped** (COMP-007) | `paused{kill_switch}`; live children finish the step and suspend; no proposals into ring 2 can be created (AGENT-013 §6.8) | automatically on reset, with a digest line saying it happened |
| Tier profile withdrawn or degraded | `paused{tier}`; children in flight finish | the profile is restored or the user picks another frontier profile |
| Session-level tool breaker (§6.6) | `paused{platform_defect}`; a `platform_defect` entry is filed | the defect is closed or the user resumes with the tool excluded |
| **Stall** (§7.1) | `stalled{diagnosis}`; digest; nothing runs | human reviews the diagnosis and resumes, amends the charter, or stops |

### 7.1 The stall detector

Evaluated after every finished run over the last `K = max(6, 20% of runs)` runs:

- **barren streak**: `knowledge_units = 0` in ≥ 4 of the last K, or the last 3 consecutively;
- **degenerating**: every one of the last K items came from `refinement_followup`, and none produced an `established` finding or a new failure class — the programme is adjusting, not predicting;
- **circling**: ≥ 2 of the last K were `proceed_as_variant` of the same family with the same failure class — the dedupe did its job and the policy overrode it;
- **cost drift**: `cost_per_knowledge_unit` over the last K exceeds 3× the session's own median (only once ≥ 2K runs exist).

Any one trips `stalled` with the diagnosis naming which, the items involved, and a suggested charter amendment (exclude the family; widen scope; a lower `delta_practical` suggestion is **never** offered — the effect size is the user's). A stall is not a failure of the session; it is the session noticing the thing a human running it would have noticed over lunch, and it is the mechanism behind "correctly reject weak ones" at the level of whole lines of inquiry.

## 8. Tiers and autonomy

- **Continuous research runs on frontier profiles only** (user decision, 2026-09-15). Local tiers run Chat and bounded Research with a human watching; they are not left alone with a budget over days. The charter form refuses a non-frontier profile with the reason. *Revisit trigger:* a local profile whose `judgement` suite (AGENT-009 §3) passes for every permitted workflow on three consecutive scorecards; opening it is a spec amendment, not a setting.
- The frontier profile must still be `measured` (AGENT-009 §5) and pass `judgement` for every workflow in `permitted_workflows`; a workflow it fails is removed from the charter's permitted set with a note, not silently run.
- `autonomy` (**default `notify`**, user decision 2026-09-15):
  - `supervised` — every item waits in *Needs you* before it starts;
  - `notify` — items start; the human is notified per start and per verdict;
  - `autonomous` — items start; the digest is the only notification.
  In all three: `SealedHoldout` and `Promotion` ask, `Spend` asks above the ask line (AGENT-008 §4), and ring 3 does not exist. `autonomous` is refused (not ignored) while the profile's `judgement` pass rate is below the profile's threshold.

## 9. The desk (AGENT-012 §2b holds the full text)

The user sees **two** options — Chat / Research — and, under Research, a **Continuous** toggle. Nothing in the desk is called a program.

- **Composer:** with Continuous off, the question field and workflow chip; with it on, the question field becomes the **charter form** (scope chips, objective, seeds, permitted workflows, exclusions, budget pre-filled from policy, schedule grid, autonomy, *keep generating questions*, `review_every`).
- **The session's conversation** is the journal: digests as cards, the human's instructions, `/agenda`, `/pause`, `/charter`. Its right panel gains two tabs when continuous: **Agenda** (the board: proposed · queued · running · done · vetoed; drag = `user` priority event; veto; add; score components and source badges on each card; the exploration pick marked) and **Charter** (versioned, amendments listed). The header shows a **continuous** badge and the day budget bar.
- **Child conversations** are grouped under the journal in the rail and carry a chip back to their agenda card.
- **Desk home:** a continuous session appears in *Running now* as one row with its next item, agenda depth and day budget; a stalled one appears in *Needs you* with its diagnosis and three actions (resume · amend charter · stop). There is no separate page.

## 10. Interfaces

- **Tables** (Phase 11 migration range, MASTER §5): `research_sessions`, `session_events (session_id, seq, kind, payload_json, created_at)` with a BEFORE INSERT transition trigger (ADR-P2-04's discipline), `agenda_items`, `session_digests`; selections go to `mlops.decision`. A bounded Research run creates a `research_sessions` row with `continuous = false` and one agenda item, so every Research conversation has the same shape.
- **Job**: `JobKind::ContinuousResearch`; children via the conversation/turn path with `session_id` and `agenda_item_id` on `agent_runs` (AGENT-010 §4 amended).
- **API**: `GET/POST /api/research/sessions`, `GET …/{id}`, `POST …/{id}/{pause|resume|stop}`, `GET/POST …/{id}/agenda`, `POST …/{id}/agenda/{item}/{approve|veto|prioritise}`, `PUT …/{id}/charter`, `GET …/{id}/digests`, `GET …/{id}/events` (SSE). Agent tokens hold `research_sessions:read` only, plus `lifecycle.open_question`/`propose` as the write path *into* an agenda (a run proposes; the driver inserts).
- **Tools** (read-only, in the `lifecycle` namespace — AGENT-013 §8; `lifecycle` goes from 9 to 12, `research` is untouched): `lifecycle.get_session(id)`, `lifecycle.list_agenda(id, status?)`, `lifecycle.get_digest(id, n?)` — so a child run can see its siblings' work when the brief's five lines are not enough.

## 11. Build order

| Phase | Items |
|---|---|
| 1 | `ResearchSession`, `Charter`, `AgendaItem`, `session_events` + trigger; bounded sessions as the one-item case; `JobKind::ContinuousResearch` with the fold and `submit_child_once`; sources 4.1, 4.2; `replenish = false` only; `supervised` only |
| 2 | scoring + exploration + `decision` rows; sources 4.3–4.6, 4.8; leases; harvest; the digest |
| 3 | pause/stall/stop table; the stall detector; kill-switch and tier coupling; `notify` (default) / `autonomous` behind the `judgement` threshold |
| 4 | source 4.7 subscriptions (profiler, coverage, monitor); replenish via `survey`; concurrency > 1 |
| 5 | the desk: toggle, charter form, Agenda and Charter tabs, rail grouping, home rows; `lifecycle.get_session/list_agenda/get_digest`; `judgement` suite cases for selection |

## 12. Acceptance

1. Killing the session driver between claiming an item and recording `run_started`, then restarting, yields one child run, one lease, and a fold identical before and after (AT-59's form).
2. Two continuous sessions with overlapping scope do not run the same question: the second insert extends the first item's sources; a forced duplicate is caught at intake by `hypothesis_precheck` and the item is `superseded`.
3. Over 40 selections with `exploration_share = 0.15`, between 3 and 9 are `exploration_flag = true` and each has a `decision` row whose `candidate_set` contains every scored item; the share cannot be set below 0.10 (the constructor refuses; the column CHECKs).
4. A child's `BudgetLease` cannot exceed the session's remaining day budget; three concurrent children cannot together exceed it; a child that exhausts its lease ends with the report turn and the session continues.
5. Closing the schedule window suspends a live child at a step boundary with no lost effect and no token spent until the next window; the next window resumes it before claiming anything new.
6. Tripping the kill switch pauses every running continuous session within one monitor event, and no `deploy_paper` proposal can be created while paused; reset resumes them and the digest records both.
7. A fixture of six barren runs trips `stalled{barren_streak}` with the six items named; a fixture of refinement-only items with no established finding trips `stalled{degenerating}`; a stalled session claims nothing until a human acts.
8. A retry condition flipping true (a fixture adapter appearing) inserts an agenda item from source 4.4 within one iteration; a regime flip in the instrument profile inserts one from 4.7 exactly once per flip.
9. With `replenish = true` and a drained agenda, a `survey` run executes and its proposed questions appear as items with its value estimates; with `replenish = false` the session is `complete`.
10. A charter on a non-frontier profile is refused at creation with the profile named; setting `autonomous` below the `judgement` threshold is refused, not ignored; a new session's `autonomy` defaults to `notify` and its day budget is pre-filled from policy.
11. The digest for a period lists every finished run's verdict, every proposal awaiting, and cost per knowledge unit, and every line resolves on click (AGENT-012 P9's test).
12. A `user` agenda item bypasses scoring and is the next claim; a veto on a queued item prevents it from ever being claimed and is visible in the digest; a bounded Research run has a `research_sessions` row with `continuous = false` and exactly one agenda item.

## 13. Non-goals

- Learning the selection policy from `decision` rows in this spec. The rows are the dataset; the learner is a later spec with its own eval.
- Multi-user sessions or shared agendas (single-operator platform).
- A session approving anything. It proposes; a human decides.
- Replacing `mlops.campaign`. A session's `optimise` item *is* a campaign; the campaign owns its phases.
- A third mode. The user chose two with a toggle; the object model keeps the continuous machinery distinct so the toggle stays honest.

## 14. Decisions taken with the user (2026-09-15)

- **Default autonomy `notify`;** no forced `supervised` first window.
- **Exploration floor 0.10** stands (explained in §5.2).
- **Day budget has a platform default** ($25/day, from the desk budget policy) and is pre-filled, not required.
- **No unattended local tiers:** continuous research is frontier-only, with the revisit trigger in §8.
- **Two modes with a toggle**, not three modes: "Program" is gone from every user-facing surface; the internal object is `ResearchSession { continuous }`.
