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

Each arrow is an event; **each child is created exactly once per `(session_id, agenda_item_id)`**, so a driver killed between "start run" and "record started" re-derives the same child rather than starting a second (AT-59's guarantee, reused as a property rather than as a call). §10 names the mechanism that delivers it. `max_concurrent_runs` > 1 means the loop may claim more than one item per iteration; each child is its own conversation (AGENT-010 §4.1: one live turn per conversation), so concurrency never shares a workspace.

> **Amended 2026-09-22 (ADR-D-06).** This sentence read *"each child is created
> through `JobStore::submit_child_once` keyed on `(session_id, agenda_item_id)`"*,
> which named a mechanism the child path does not and will not use, and contradicted
> §10's *"children via the conversation/turn path"* in the same document. §10 is the
> one that exists: **a conversation turn is not a job.** It is started by
> `AgentManager::send_message`, it runs on a spawned task, and its identity is a row
> in `agent_runs` rather than in the job store — so `submit_child_once`, which keys on
> `(parent job, member index)` and hands back a job, has nothing to hand back for one.
> Following §3 literally would have meant building a worker that does not exist in
> order to satisfy a sentence, and breaking the desk, the rail and the steer controls,
> all of which read the conversation path today. **What §3 was reaching for is the
> exactly-once property**, which is real, is what the sentence now states, and is
> delivered by the derived conversation id plus the conditional claim (ADR-P11-01) —
> proven by two restart tests, `restarting_between_start_and_record_re_derives_the_same_state`
> and `a_child_conversation_id_is_the_same_one_after_a_restart`.

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
- **cost drift**: `cost_per_knowledge_unit` over the last K exceeds 3× the median of the runs *preceding* the window (still the session's own history, not a platform figure; only once ≥ 2K runs exist).

> **Amended 2026-09-22 (ADR-D-07).** The cost-drift rule read *"exceeds 3× the
> session's own median"*, with no statement of which runs the median is taken over.
> Read literally — the median over **every** run, the judged window included — the
> rule **cannot fire at `2K` runs, the exact point its own gate opens.** At `2K` the
> window is half the session, so with K cheap runs at `c` and K dear runs at `d` the
> median is `(c + d) / 2`, and `d > 3 · (c + d) / 2` reduces to `−d > 3c`, which has
> no solution for positive costs. Ten times, a thousand times, a million times the
> baseline all fail. That is not a stricter reading of the spec; it is a sentence with
> an arithmetic error in it, and the inertness is kept as a test over three orders of
> magnitude —
> `the_baseline_is_the_history_because_a_window_inside_its_own_median_can_never_drift`
> in `crates/research-session/src/stall.rs`. The baseline is therefore the history
> **before** the window, which is what is built and what makes the rule live from `2K`
> onward as the gate intends. The other three rules have absolute thresholds and
> self-gate; only this one was affected.

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
| 1 | `ResearchSession`, `Charter`, `AgendaItem`, `session_events` + trigger; bounded sessions as the one-item case; `JobKind::ContinuousResearch` with the fold and the exactly-once child over the conversation path (§3, §10); sources 4.1, 4.2; `replenish = false` only; `supervised` only |
| 2 | scoring + exploration + `decision` rows; sources 4.3–4.6, 4.8; leases; harvest; the digest |
| 3 | pause/stall/stop table; the stall detector; kill-switch and tier coupling; `notify` (default) / `autonomous` behind the `judgement` threshold |
| 4 | source 4.7 subscriptions (profiler, coverage, monitor); replenish via `survey`; concurrency > 1 |
| 5 | the desk: toggle, charter form, Agenda and Charter tabs, rail grouping, home rows; `lifecycle.get_session/list_agenda/get_digest`; `judgement` suite cases for selection |

> **Amended 2026-09-22 (ADR-D-06).** Phase 1 read *"`JobKind::ContinuousResearch`
> with the fold and `submit_child_once`"*. It named the same struck mechanism §3 did,
> so a builder reading the build order rather than §3 would have rebuilt the job-backed
> child the ADR refused. Nothing about the phase's contents or its position changed —
> only the name of the path the child is created on.

## 12. Acceptance

> **Form note (Set S task S-0.4, 2026-10-03).** Converted from a plain numbered
> list to the repository's acceptance form so `docs/procedures/execute-plan.md`
> step 3c can close the chain requirement -> criterion -> evidence. No criterion
> was added, removed, reworded or softened; each kept its position and its number
> (`1a` is `AC-1a`), so item *n* of the old list is `AC-n`. An unticked box with
> `Verified by: [--]` means nothing has been proved, which is the honest default.
> The form is the one AGENT-005, AGENT-006 and AGENT-008 use.

- [ ] AC-1: Killing the session driver between claiming an item and recording `run_started`, then restarting, yields one child run, one lease, and a fold identical before and after (AT-59's form). - Verified by: `research-session::fold::tests::{restarting_between_start_and_record_re_derives_the_same_state,a_finish_the_log_never_started_is_an_anomaly_and_stops_claiming,a_hole_in_the_log_is_an_anomaly}` (the fold is a pure function of the log), `api::session_driver::tests::{a_claim_the_log_never_started_is_stale_and_one_it_did_is_not,an_item_already_started_is_not_claimed_again}` and against Postgres `api/tests/pg_session_leases.rs::a_restarted_driver_records_one_lease_for_the_one_child`. **Partial:** the restart is a replay of the log, not a killed process; one child run, one lease and an identical fold are each asserted separately. Written, not re-run in this pass (needs live Postgres or a full crate build); CI runs it. *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-2: Two continuous sessions with overlapping scope do not run the same question: the second insert extends the first item's sources; a forced duplicate is caught at intake by `hypothesis_precheck` and the item is `superseded`. - Verified by: `research-session::agenda::tests::{a_second_source_extends_the_first_item_instead_of_inserting,the_key_ignores_wording_noise_but_not_wording}`, `research-session::replenish::tests::two_surveys_of_one_charter_version_are_one_question`, `research-session::sources::tests::two_subscriptions_wanting_the_same_sentence_make_one_item_with_two_sources`, `api/tests/pg_session_stores.rs::two_sessions_with_one_question_produce_one_item_with_two_sources`, and the three-layer scorecard `research-session::agenda::tests::{the_scorecard_counts_layer_one_off_the_offers_it_was_given,the_scorecard_read_off_the_log_says_which_layer_caught_each_duplicate}`. **Partial:** the forced duplicate caught at intake by `hypothesis_precheck` and the item becoming `superseded` rests on the precheck's recorded verdict (`research-session::select::tests::{a_recorded_precheck_multiplies_novelty_and_an_absent_one_does_not,an_unrecorded_precheck_is_not_a_proceed_new}`); `hypothesis_precheck` filing `blocked | duplicate | new` is absent in the common case (S-11.4 note). Written, not re-run in this pass (needs live Postgres or a full crate build); CI runs it. *(Filled 2026-10-04, S-8.9.)*
- [x] AC-3: Over 40 selections with `exploration_share = 0.15`, between 3 and 9 are `exploration_flag = true` and each has a `decision` row whose `candidate_set` contains every scored item; the share cannot be set below 0.10 (the constructor refuses; the column CHECKs). - Verified by: `research-session::select::tests::{the_exploration_share_is_achieved_over_forty_selections,the_achieved_share_is_the_declared_share,the_floor_is_achieved_and_not_merely_permitted,the_propensities_of_the_two_branches_sum_to_one,the_same_seed_replays_the_same_choice,the_stored_score_is_the_one_the_ranking_used_and_not_the_raw_signals}`, the floor `research-session::charter::tests::the_floor_holds_on_every_way_in`, `invariants/tests/exploration_floor_is_one_number.rs::{the_constructor_refuses_what_the_column_refuses,the_migration_checks_the_number_rust_refuses_below}` (migration `0138`'s CHECK reads `exploration_share >= 0.10`), and the decision row `invariants/tests/a_selection_is_logged_with_what_it_rejected.rs::{the_decision_carries_the_whole_candidate_set_and_the_propensity,choosing_writes_a_decision_row_as_well_as_an_event}`. Written, not re-run in this pass (needs live Postgres or a full crate build); CI runs it. *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-4: A child's `BudgetLease` cannot exceed the session's remaining day budget; three concurrent children cannot together exceed it; a child that exhausts its lease ends with the report turn and the session continues. - Verified by: `research-session::execute::tests::{a_lease_never_exceeds_the_remaining_day_budget,three_concurrent_children_cannot_together_exceed_the_day_budget,a_lease_with_no_real_ceiling_at_all_is_refused,a_lease_that_would_buy_nothing_is_refused,a_day_in_trials_caps_a_lease_and_a_spent_day_refuses_one}`, `api::session_driver::tests::{the_stored_spend_unit_is_the_unit_the_lease_is_metered_in,a_finished_run_with_no_recorded_trials_makes_the_spend_unknown_and_refuses_the_lease}`, `api/tests/the_session_driver_calls_the_modules_it_was_given.rs::a_refused_lease_starts_no_child`, against Postgres `api/tests/pg_session_leases.rs::{three_live_leases_are_summed_and_a_finished_one_is_not,a_lease_is_committed_in_full_and_released_once_and_never_deleted}`, and a child that exhausts its lease is refused further steps and tokens (`api::agent::subrun::tests::a_persisted_lease_refuses_past_its_steps_and_tokens_and_a_missing_axis_is_not_zero`) and gets one report turn (`harness::drive::tests::a_spent_token_budget_buys_one_report_turn_and_says_so`). The unit is trials (USD only when priced); an undenominated total cannot stop a session and says so (`an_undenominated_total_cannot_stop_a_session_and_says_so`). **Partial:** the session *continuing* after a child's lease is exhausted is not asserted by a test that runs both. Written, not re-run in this pass (needs live Postgres or a full crate build); CI runs it. *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-5: Closing the schedule window suspends a live child at a step boundary with no lost effect and no token spent until the next window; the next window resumes it before claiming anything new. - Verified by: `research-session::execute::tests::{the_next_window_resumes_before_it_claims_anything_new,a_closed_window_drains_the_live_children_and_re_arms_on_the_opening,an_open_window_hands_the_live_children_their_boundary,a_resumed_child_occupies_a_slot,an_indeterminate_calendar_holds_rather_than_claiming}`, `research-session::schedule::tests::{the_openness_of_an_instant_says_when_the_child_drains,a_local_hour_that_never_happens_does_not_open_a_window,the_re_arm_is_never_an_instant_already_past}` and `api::session_driver::tests::{the_gate_closes_outside_the_window_and_names_when_it_re_arms,a_drained_child_is_suspended_until_the_log_says_it_was_resumed}`. **Partial:** "no lost effect and no token spent until the next window" combines the drain (proved as a plan and as driver tests) with the loop's suspend (`harness/tests/waiting_costs_nothing.rs`); no test runs a live child across a window boundary. *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-6: Tripping the kill switch pauses every running continuous session within one monitor event, and no `deploy_paper` proposal can be created while paused; reset resumes them and the digest records both. - Verified by: `research-session::couplings::tests::{a_tripped_switch_pauses_and_names_itself,a_reset_switch_resumes_without_anything_else_having_to_happen,an_unreadable_switch_is_not_an_armed_one,no_ring_two_proposal_is_created_while_the_switch_is_tripped,the_kill_switch_drains_and_a_degraded_tier_lets_children_finish}`, `api::session_driver::tests::{an_unread_kill_switch_reaches_the_module_as_neither_armed_nor_tripped,a_failed_read_holds_the_iteration_without_pausing_the_session}`, `api/tests/pg_session_stores.rs::a_monitor_notification_wakes_a_session_within_one_event_and_the_heartbeat_does_not`, and the digest line `research-session::digest::tests::a_stall_writes_a_pause_line_naming_the_rule_and_what_to_do`. **Partial:** one monitor event waking a session is asserted against the notification channel; a real kill-switch trip across a running session, then a reset, is not run. Written, not re-run in this pass (needs live Postgres or a full crate build); CI runs it. *(Filled 2026-10-04, S-8.9.)*
- [x] AC-7: A fixture of six barren runs trips `stalled{barren_streak}` with the six items named; a fixture of refinement-only items with no established finding trips `stalled{degenerating}`; a stalled session claims nothing until a human acts. - Verified by: `research-session::stall::tests::{six_barren_runs_trip_the_streak_and_name_all_six,three_barren_in_a_row_trip_it_even_when_the_window_is_mostly_productive,two_barren_runs_are_not_a_streak,refinement_only_runs_that_establish_nothing_are_degenerating,one_established_finding_rescues_the_window_from_degenerating,the_stalled_event_carries_the_word_the_fold_reads_and_a_stalled_session_claims_nothing,the_guard_against_lowering_the_bar_would_catch_a_violation}` and `api::session_driver::tests::a_barren_stretch_of_rows_reaches_the_detector_and_trips_it`. A stalled session claims nothing until a human acts (`api::session_driver::tests::a_session_that_is_not_running_claims_nothing`). *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-8: A retry condition flipping true (a fixture adapter appearing) inserts an agenda item from source 4.4 within one iteration; a regime flip in the instrument profile inserts one from 4.7 exactly once per flip. - Verified by: `research-session::sources::ledger::tests::{a_fired_condition_inserts_an_investigate_naming_the_class_it_died_of,the_window_extending_fires_and_the_window_standing_still_does_not,an_unmeasured_subject_is_not_evaluable_rather_than_false,exactly_one_class_in_ten_can_actually_be_evaluated}` (source 4.4) and `research-session::sources::observe::tests::{one_flip_read_by_three_profiler_runs_is_one_item,a_second_flip_is_a_second_item_even_on_the_same_subject,a_blip_is_not_something_the_session_reacts_to}` (source 4.7), `api/tests/pg_session_stores.rs::a_regime_flip_has_one_identity_across_three_reads`. **Partial:** only one retry class in ten has an evaluable condition today (the rest are a *measured no*), and the regime producer is `session_regime::flips` over `regime_causal.regime_state` (S-11.10); a fixture *adapter appearing* inserting an item has no test. Written, not re-run in this pass (needs live Postgres or a full crate build); CI runs it. *(Filled 2026-10-04, S-8.9.)*
- [x] AC-9: With `replenish = true` and a drained agenda, a `survey` run executes and its proposed questions appear as items with its value estimates; with `replenish = false` the session is `complete`. - Verified by: `research-session::replenish::tests::{a_drained_agenda_with_replenish_off_completes,a_drained_agenda_with_replenish_on_runs_a_survey,a_surveys_proposals_become_scored_items_under_their_own_source,a_proposal_with_no_estimate_does_not_get_one,a_proposal_outside_the_charters_scope_is_refused_with_a_line}`, `api::session_driver::tests::{a_drained_agenda_ends_the_session_only_when_it_is_not_set_to_keep_going,an_absorbed_survey_is_never_left_out_for_having_no_trials_and_its_yield_is_the_explicit_count}` and `api::session_survey::tests::a_report_carries_each_proposal_with_its_estimate_and_never_an_invented_one`; stored with their estimates by `api/tests/pg_session_stores.rs::a_surveys_proposals_are_stored_with_their_estimates_and_a_deduped_survey_is_an_explicit_zero`. *(Filled 2026-10-04, S-8.9.)*
- [x] AC-10: A charter on a non-frontier profile is refused at creation with the profile named; setting `autonomous` below the `judgement` threshold is refused, not ignored; a new session's `autonomy` defaults to `notify` and its day budget is pre-filled from policy. - Verified by: `research-session::charter::tests::{a_local_profile_is_refused_by_name_with_the_reason,an_unmeasured_frontier_profile_is_refused_too,autonomous_below_the_profiles_own_threshold_is_refused_not_ignored,a_new_session_defaults_to_notify,a_day_budget_is_pre_filled_rather_than_required,the_threshold_is_the_profiles_own_and_not_a_constant,a_judgement_nobody_measured_has_no_rate_to_compare}`, `api::session_profile::tests::{autonomous_is_granted_on_a_recorded_passing_judgement_and_refused_below_the_line,a_rate_is_judged_against_the_line_it_was_measured_against}` (`profile_judgement`, migration `0163`) and `frontend/src/components/desk/__tests__/modes.test.tsx` ("refuses a non-frontier profile inline, naming it"). The `judgement` rate that `autonomous` is compared against is written by the platform from a recorded scorecard; the S-10.16 suite is what produces one once a live run exists. *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-11: The digest for a period lists every finished run's verdict, every proposal awaiting, and cost per knowledge unit, and every line resolves on click (AGENT-012 P9's test). - Verified by: `research-session::digest::tests::{every_line_carries_an_id_that_opens_something,a_run_is_reported_by_its_verdict_and_not_by_its_title,the_budget_line_reports_cost_per_knowledge_unit,only_the_proposals_still_waiting_are_listed,the_next_five_are_five_with_their_scores_and_why,writing_the_same_period_twice_gives_the_same_digest}`, `api::session_driver::tests::{the_period_is_everything_since_the_last_digest_and_nothing_before_it,the_digest_is_stored_saying_which_unit_its_numbers_are_in}`, `frontend/src/components/desk/__tests__/{a-digest-link-opens-what-it-names,a-digest-number-is-not-dollars,away-digest}.test.tsx`. **Partial (S-11.12 `PARTIAL`):** every line carries an id and links resolve per kind; cost per knowledge unit is a count of units, and USD only when priced (`a-digest-number-is-not-dollars.test.tsx`). *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-12: A `user` agenda item bypasses scoring and is the next claim; a veto on a queued item prevents it from ever being claimed and is visible in the digest; a bounded Research run has a `research_sessions` row with `continuous = false` and exactly one agenda item. - Verified by: `research-session::agenda::tests::user_is_the_only_source_that_skips_the_scorer`, `research-session::select::tests::a_user_item_is_taken_without_being_drawn`, `api::session_driver::tests::{a_user_item_bypasses_the_scorer_and_goes_next,a_vetoed_item_is_never_claimed_even_if_its_row_says_queued,an_item_that_has_not_been_approved_is_not_claimable}`, `research-session::fold::tests::a_veto_survives_every_later_event`, and a bounded run: `research-session::tests::{a_bounded_session_is_one_item_from_the_user,the_shape_rule_refuses_both_halves_and_neither}` (the shape rule). **Partial:** that the *route* writes a `research_sessions` row with `continuous = false` and exactly one agenda item for a bounded Research run is the pure shape rule here, not an observed row. *(Filled 2026-10-04, S-8.9.)*
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
