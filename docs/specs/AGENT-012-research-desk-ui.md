# AGENT-012: The Research Desk — the product spec for working with the agent

**Status:** Proposed (product + implementation spec; ready to build)
**Date:** 2026-09-14
**Author:** Claude (Fable 5.1), as product designer
**Supersedes:** COMP-006 (research workspace UI) §2–§6; the current `/agent` page (`frontend/src/pages/AgentPage.tsx`, `components/agent/{ChatView,ConversationSidebar,AgentActivity}.tsx`)
**Design system:** Meridian (`docs/meridian-design-system/01-DESIGN-SPEC.md`) is normative for tokens, components, states, themes and motion; this spec adds screens and components to it and does not restate its foundations
**Depends on:** AGENT-005…011 for the data every screen shows; COMP-007 for the live monitor; the APIs named in §12

---

## 0. What this is, and what exists today

Today `/agent` is a text field, a conversation list with archive, and a timeline that renders run events by kind (`ChatView.tsx:126-276` handles `user`, `assistant`, `tool_call`, `tool_result`, `canary`, `plan`, `exposure`, `policy`, `validation`, `core_tool`, `compaction`, `usage`, `degradation`, `status`, `error`, `final`). A separate `/approvals` page mounts an inbox. A `/research/:projectId/sessions/:sessionId` workspace exists with panes, a timeline, a budget meter and a steering input (`components/workspace/*`). Three surfaces, none of which lets a person *see what the agent knows, decide what it may do, or judge what it found* without reading raw events.

The Research Desk is one surface. Its thesis: **a human and an agent researching together should share one desk** — the same working state, the same evidence, the same budget, the same controls — and the human should never have to read a transcript to know where things stand.

### 0.1 Who it is for

- **The quant** (primary): knows the domain, wants leverage, distrusts anything unexplained. Reads evidence before conclusions. Will use keyboard everything.
- **The operator** (same person, different hour): wants to know what is running, what it costs, what is waiting on them, and that nothing is trading that shouldn't be.
- **The reviewer** (later, or the quant on Monday): opens a dossier cold and needs to judge it in five minutes.

### 0.2 Principles (added to Meridian §1.3)

8. **State over transcript.** The primary view of a run is its working state (goal, plan, evidence, findings, budget), not its message log. The log is one click away and never hidden.
9. **Every number is a link.** An id, a Sharpe, a trial count is clickable and opens the thing it names: the experiment, the study, the ledger row, the bars. If it cannot be opened it is not evidence, and the UI shows it as prose, not as a number.
10. **The human's controls are always visible and always work.** Pause, steer, approve, deny, escalate, stop — one row, never scrolled away, on every tier, including while the agent is mid-call.
11. **Cost and safety are ambient.** Budget, cache hit, trials spent, environment (paper/live) and the kill-switch state are in the chrome, not in a report.
12. **Understanding is two-way.** The agent shows what it decided and why; the person can ask "why?" on any card and correct any decision. Corrections become memory (AGENT-007 §3.4).

## 1. Information architecture

```
/desk                         Desk home: inbox, running, recent, pinned findings, tier status
/desk/c/:conversationId       Conversation (the main working screen)
/desk/c/:id/run/:runId        Run detail: working state + trace + replay scrubber
/desk/c/:id/dossier/:ticket   Dossier reader (also /desk/dossiers)
/desk/approvals               Global approvals inbox (keeps /approvals as alias)
/desk/memory                  Memory browser: findings, negative results, profiles, preferences, playbooks
/desk/catalogue               Tool catalogue and workflows; what each tier sees
/desk/models                  Models, providers, tiers, calibration and scorecards
/desk/monitor                 Live strategies, control limits, kill switch (COMP-007)
/desk/usage                   Budgets, spend, cache, per-run costs
/desk/settings                Providers, keys, defaults, density, theme, notifications
```

The top bar (Meridian §3.19) gains a **Desk** entry and three ambient indicators: *agent activity* (running/paused/waiting count), *approvals due* (badge), and *kill-switch state* (armed/halted). The environment pill (paper/live) stays exactly as Meridian §5.4 specifies.

**Layout.** A three-column desk on wide screens: left rail (conversations + filters, collapsible to icons), centre (the conversation), right panel (working state, switchable to evidence, memory, or catalogue). At <1200px the right panel becomes a drawer; at <768px the rail becomes a sheet and the conversation is full-bleed with the controls row docked to the bottom. Both Meridian densities apply; `compact` is the default for `terminal`.

## 2. Desk home (`/desk`)

A dashboard for the operator hour. Panels, in reading order:

1. **Needs you** — approvals, questions from the agent (`ask_user`), runs paused for a decision, runs escalated to a tier you must confirm, **proposals** into rings 1 and 2 with their envelope and the tickets that licensed them (AGENT-013 §7.5), and **stalled continuous sessions** with their diagnosis (AGENT-014 §7.1). Each row: what, from which conversation, how long waiting, the two obvious actions inline. Empty state: "Nothing is waiting on you" with the last decision you made.
2. **Running now** — every active run: conversation title, tier badge (`frontier / local_high / … / degraded`, plus `unmeasured` when calibration is missing), current step (from the working state), what it is waiting on (model, tool, job with progress, human), tokens and dollars so far, cache hit %, a **Pause** and **Stop** per row.
2a. **Since you were away** — the operator persona's actual question, which no other panel
   answers: the desk knows when you last looked, so it can say what changed. One block, above
   the fold, covering the gap since your last visit — runs that finished and what they
   concluded (verdicts, not titles), findings established and any marked `refuted`, approvals
   that timed out and what defaulted, monitor state changes and anything scaled down or
   halted, budget spent, and anything now waiting on you. Each line opens the thing it names.
   Dismissable, resets the marker, never shown for a gap under an hour. A research agent that
   runs for hours without you is only useful if returning to it takes a minute, and a
   reverse-chronological event list is not that — it makes you reconstruct a state the
   platform already knows.
3. **Recent conclusions** — last N findings across conversations, as evidence chips (§6.3), filter by instrument/family/confidence, pin to keep.
4. **The book** — a compact mirror of `/desk/monitor`: strategies live/paper, control-limit status per strategy, the kill-switch state, one **Halt all** button behind Meridian's irreversible-action pattern.
5. **Tier status** — one tile per model profile: measured/unmeasured, decode ladder, last scorecard, calibrated bytes/token, "run a check" action.
6. **Start** — a large input ("Ask, or describe what to research"), with model picker, instrument/timeframe chips, and a **mode** control (*Chat / Research*, with the **Continuous** toggle under Research — AGENT-013 §2, §2b) and three starters (ask a question, research an edge, start continuous research).

## 2b. Continuous research in the desk — the toggle and what it turns on

Added 2026-09-14, revised 2026-09-15 (AGENT-014 §9; ADR-0037 §2). The user sees **two** options
— Chat / Research — and, under Research, a **Continuous** toggle. Nothing in the desk is called
a program, and there is no separate page: a continuous session *is* a conversation.

**Composer, toggle off.** The question field, the workflow chip, the constraint chips.

**Composer, toggle on.** The question field becomes the **charter form**: scope chips
(instruments, asset classes, families, timeframes), objective picker (`find_deployable` with
the SPEC §10 constraint form · `build_knowledge` · `maintain` · mixed), seeds (question rows),
permitted workflows (defaulting to all the profile supports), exclusions, standing constraints,
budget (day and total, trials, max runs — **pre-filled from the budget policy in §12, platform
default $25/day, never required**), schedule grid (days × hours, timezone), concurrency,
autonomy (`supervised / notify / autonomous`, **default `notify`**; `autonomous` disabled with
the reason when the profile's `judgement` score is below threshold), **Keep generating
questions** (the `replenish` — endless — setting), `review_every`, and a `delta_practical`
suggestion field marked *suggestion; each run seals its own*. A non-frontier profile is refused
inline with the profile named (AGENT-014 §8). Toggling off and on again does not lose typed
constraints.

**The session's conversation is the journal.** Digests render as cards (AGENT-014 §2.4, each
line a link); the human's messages are instructions to the session; slash commands
`/agenda add …`, `/pause`, `/resume`, `/charter`. The header shows a **continuous** badge, the
day budget bar and the agenda depth. The right panel gains two tabs while continuous:
- **Agenda** — a board: *proposed · queued · running · done · vetoed*. Each card: the question,
  workflow chip, source badge(s) (a card wanted by three sources shows three), score components
  as a small bar (value · novelty · feasibility · cost) with the total and the rationale
  sentence from the decision row, the exploration mark when it was the exploration pick, and
  the child conversation link once running. Drag to reorder → a `user` priority event;
  **Veto**, **Approve** (under `supervised`), **Add**. Running cards show the child's stage
  strip (§3.2 *Stage* card) inline.
- **Charter** — the versioned charter with an amendment list; editing appends an amendment and
  the next digest names it.

**Child conversations** are grouped under the journal in the left rail and carry a chip in the
header (§3.1) back to their agenda card; opening one gives the ordinary Research view with the
full controls row.

**Desk home.** A continuous session is one row in *Running now* with its next agenda item,
agenda depth and day budget bar, plus Pause/Resume; a stalled session appears in *Needs you*
with its diagnosis and the three actions (resume · amend charter · stop).

## 3. The conversation (`/desk/c/:id`)

### 3.1 Header
Title (editable; auto-titled), model and tier badge, environment pill, **mode · workflow · ring · stage** (AGENT-013 §2–§5; the ring is the answer to "what can this run change?"), a **continuous** badge on a session's journal and a **session chip** on its child conversations (§2b), **status** (idle / thinking / calling `tool` / waiting on job 34% / waiting on you / paused / done), elapsed, cost ticker (tokens · $ · cache%), trials spent · N_eff, and the controls row (§3.4). A **Fork** action starts a new conversation from any point with the same brief; **Share** exports a read-only dossier link; **Replay** opens the run scrubber.

### 3.2 The stream
Not a chat transcript: a **stream of cards**, one per meaningful event, grouped by step with a step header ("Step 7 · sweep parameters · 2 tools · 41 s"). Raw events (exposure, compaction, validation rungs, usage) are folded under a **details** disclosure per step, off by default, on with one key (`d`). Card types:

| Card | Shows | Actions |
|---|---|---|
| **You** | your message, constraints as chips, attachments (a strategy, a dataset window, a dossier) | edit-and-resend (forks) |
| **Plan** | numbered steps with status glyphs (pending · active · done · skipped · failed) and the tool each names | reorder/skip/insert (sends a steer), "why this plan?" |
| **Tool** | name, namespace, risk badge, arguments (collapsed, pretty), duration, result **summary** (AGENT-006 §2.4) as prose with linked ids, `n`/`se` inline, handle → opens full result | rerun, open full result, "explain this result" |
| **Finding** | claim, confidence pill (`established / suggestive / refuted`), evidence ids as chips, n, window, "cited later" mark | pin, dispute (opens a correction that becomes memory), copy citation |
| **Negative result** | what died, failure class, retry condition | "retry when…" reminder |
| **Decision** | what the agent chose and why (AGENT-008 §3.6) | override (sends a steer and records a preference) |
| **Question** | the agent's `ask_user`, with the options it proposed | answer inline; "remember this" toggle |
| **Approval** | the action, its envelope (`Spend / Promotion / SealedHoldout / Tier3Memory`), what it would do, what it costs | approve / deny / approve-with-note; irreversible ones take the two-step pattern |
| **Job** | sweep/study/training/shadow progress bar, ETA, the run ids as they appear | open in the research workspace, cancel |
| **Gate** | funnel position, the tickets that licensed the advance (leakage audit, selection-bias verdict, battery), which are missing | open each ticket |
| **Escalation** | from tier → to tier, the working state carried, why | confirm/decline |
| **Correction** | a validation rejection the agent received, in plain words, and what it did next | — |
| **Compaction** | "3 results moved to files; 2 summarised", with handles | open the files |
| **Report** | the `finish_task` result, evidence, assessment; opens the dossier | open dossier, share, start follow-up |
| **Stage** (AGENT-013 §5) | the ten-stage strip with the current stage lit and, under it, *what would move it* (the gate not yet met, in words) | open the gate's ticket, steer |
| **Pre-registration** (AGENT-013 §5.3) | objective, benchmark, `delta_practical`, window, embargo, costs, null family, trial budget, the falsifier, the hash — rendered *before* any trial is spent; amendments shown as lineage | steer (creates an amendment with the lineage warning), copy hash |
| **Research offer** (AGENT-013 §2.2) | in Chat: the question as understood, the workflow, estimated cost and trials, the registration it would seal | **Start** (opens a Research run), edit question, "always start" for this scope |
| **Proposal** (AGENT-013 §7.5) | kind (promote · deploy paper · adopt playbook · change limits), ring, what changes, reversibility, cost, the dossier and the tickets that license it, the envelope | approve / deny / approve-with-note (irreversible ones take the two-step pattern); refused-while-halted shows the reason |
| **Halt / degradation** | fenced, stuck, budget exhausted, backend failure — with the one thing the person can do | resume / retry / escalate |

Every id anywhere is a chip (§6.3). Hovering a chip previews the object; clicking opens it in the right panel; `⌘/Ctrl-click` opens the full page.

### 3.3 Composer
Multi-line input with: model picker (profiles, with tier badge and "unmeasured" warning), **constraints** row (instrument, timeframe, window, balance — chips with pickers; they render as the opening message, AGENT-005 §3.2), **attach** (a strategy version, a dossier, a finding, a file from the workspace), **mode** segmented control (AGENT-013 §2): *Chat* (no trial can be spent) and *Research* (the lifecycle, with a **workflow** chip — investigate · falsify · replicate · compare · diagnose · survey · optimise — defaulted from the question's shape and overridable). Under Research, a **Continuous** toggle: on, the question field becomes the charter form (§2b). **Write strategy** is an action beside *attach* (opens the SLv2 editor, §8), not a mode. Slash commands: `/steer`, `/pause`, `/pin`, `/why`, `/model`, `/budget 5$`, `/tier local`. Sending while a run is active is a **steer** by default (AGENT-010 §7), shown as a distinct card.

### 3.4 Controls row (always visible)
`Pause` · `Resume` · `Stop` · `Steer…` · `Approve/Deny` (when pending) · `Escalate to…` · `Budget` (edit the cap for this run) · `Details` toggle · `Replay`. Keyboard: `space` pause/resume, `esc` stop (confirm), `a`/`x` approve/deny the focused approval, `s` steer, `d` details, `r` replay, `/` focus composer, `?` shortcuts.

### 3.5 Right panel (tabs)
- **State** (default): the `WorkingState` card (AGENT-008 §2) — goal and kind, acceptance criteria with ticks, hypothesis and status, plan tracker, evidence list (ids with the tool that produced them), findings, open questions, decisions, last error, **budget meter** (steps/tokens/$ /trials, with the profile's caps and the 90% wrap-up line).
- **Evidence**: every id this run produced, grouped by type, with open-in-place viewers: experiment summary, study surface (the plateau/spike plot), gate ledger, trade list with the concentration bar, bars chart for a window, data-quality probe results.
- **Memory**: what the brief contained (AGENT-007 §4) — preferences, instrument profile, findings and negative results retrieved, each marked *cited* when the run used it; "why retrieved" on hover; **add to memory** and **forget** actions.
- **Tools**: what this tier can currently see (exposure), what it searched for, the whole catalogue with a search box; clicking a tool shows its notes; **pin a tool** for this run (sends a `search_tools`-equivalent).
- **Cost**: tokens by layer (charter / schemas / body / output) per call, cache hit per call as a sparkline, $ cumulative, the calibrated bytes/token and the harness's estimate vs. the provider's count.

## 4. Run detail and replay (`/desk/c/:id/run/:runId`)
The full trace (AGENT-010 §5 `agent_events`) as a scrubbable timeline: a horizontal step strip; drag or arrow keys to move; the centre shows the working state **as it was at that step** and the model call's rendered prompt sections with their token sizes (charter, brief, working state, schemas) — the same numbers AGENT-005 budgets. Toggle **prompt** to read exactly what the model saw, **reply** to read what it said, **diff** to see what changed since the previous step. This is also the eval author's tool: **Save as case** writes the trajectory to `evals/` (AGENT-009 §3).

## 5. Dossier reader (`/desk/c/:id/dossier/:ticket`, `/desk/dossiers`)
A document, not a card stack, in the `paper` theme's reading layout even when the app is in `terminal`: verdict on top; the evidence table (claim → id → n/se → verification method); the plan as executed; the funnel with tickets; the unresolved-questions and reviewer-critique sections (mandatory, AGENT-006 §5.8); cost and trials; a **provenance strip** (harness version, profile, data window, seeds). Every claim has a **verify** control that re-runs the recheck live. **Comments** anchor to claims; a comment can be sent to the agent as a follow-up turn. Export: PDF, Markdown, and a signed JSON bundle.

## 6. Shared components (added to Meridian §3)

### 6.1 Tier badge
`frontier · local_high · local_mid · local_small · degraded`, with a second glyph for measured/unmeasured (AGENT-009 §5). Tooltip: model, decode ladder, exposure budget, last scorecard date.

### 6.2 Budget meter
Four stacked bars (steps, tokens, $, trials) with the cap, the 90% wrap-up mark, and the projected end at current burn. Turns `warn` at 75%, `neg` at 90%, always with the text value (never colour alone).

### 6.3 Evidence chip
`exp_7f0c` `study 3` `gate 2` `f_31a` — monospace, type icon, hover preview, click opens. A chip that cannot resolve renders struck-through with "not found" — this is the UI's honesty check on the agent's honesty check.

### 6.3a Provenance mark — the fence has to reach the human too

The platform wraps untrusted content (fetched pages, third-party tool descriptions) in a
provenance fence so the *model* can tell data from instruction. Nothing currently carries
that distinction to the *person*, and the person is the one who acts on it. The path is
short and real: a fetched page contains a persuasive claim, the agent records a finding that
leans on it, the finding renders in the desk exactly like one derived from the platform's own
bars, and the human promotes a strategy on it. The trust boundary the harness defends is
undefended one layer up.

Every card, chip and memory row that traces to `Provenance::Untrusted` carries a **provenance
mark**: a small glyph plus the source host, and on hover the fetch time and the run that
fetched it. `evidence_class` from AGENT-007 §2.2 renders the same way — `direct_stat`,
`derived`, `model_summary` and `human` are visibly different things, and a `model_summary`
never looks like a measurement.

Two hard rules, because this is a security surface and not a styling choice:

- **Untrusted text is rendered as text, never as markup.** No HTML, no link auto-linking, no
  image loading from fetched content. A fetched page cannot put a clickable link in front of
  the operator.
- **A dossier claim whose evidence chain touches untrusted content is marked in the evidence
  table**, and the mark survives export. A reviewer reading the PDF a month later sees it.

### 6.4 Confidence pill
`established / suggestive / refuted / stale` — text always, colour second.

### 6.5 Risk badge (on tool cards)
`read / write / destructive / outbound` — outbound and destructive carry an icon.

### 6.6 Step header and details disclosure
Groups cards; shows step count, tools called, duration, tokens; disclosure reveals the raw events.

### 6.7 Envelope banner (on approvals)
Names the envelope, what the action changes, whether it is reversible, and the second step for irreversible ones (Meridian §5.5).

### 6.8 Job progress
Deterministic progress (done/planned) with ETA from the job service; never an indeterminate spinner over content (Meridian §3.22).

### 6.9 Command palette
`⌘K`: jump to a conversation, id, finding, strategy, tool, page; run a slash command; switch model/tier; toggle theme/density.

## 7. Memory browser (`/desk/memory`)
Tabs: **Findings** (table: claim, subject, confidence, n, window, valid_until, cited count, conflicts), **Negative results** (with retry conditions and a "retry now" action that opens a conversation with the brief), **Instrument profiles** (cards with the analysis summaries and a refresh action), **Open questions** (question, subject, blocking, value, opened by, status; *research this* starts a run with the question — AGENT-013 §9), **Preferences** (yours, editable, scoped), **Playbooks** (resident listing, proposed, verification status, vote, deprecate), **Signal registry**. Every row opens its provenance (which run, which tool, which data). A **conflicts** filter shows disagreeing pairs side by side with a "resolve" that starts a diagnose conversation.

## 8. Strategy authoring (inline and `/strategy-builder` link)
Inside a conversation, **Write strategy** opens a split editor: SLv2 text on the left (syntax highlighted, `list_features` autocomplete, inline validator errors with **Apply fix**), and on the right the live **preview** (AGENT-011 `preview_signals`): entries per month bars, the sample events table, warm-up status, "never fires because…" if so. Buttons: **Validate**, **Preview**, **Create version**, **Diff vs parent** (moves in words), **Hand to agent** (the agent continues from this version). The same editor is what the agent's "Strategy" cards open when clicked.

## 9. Catalogue (`/desk/catalogue`)
The AGENT-006 catalogue as a browsable surface: namespaces as columns, tools as cards with tier badges for which profiles see them, risk, trial-spending flag, output budget, notes; workflows shown as sequences with their decision points; a **simulate exposure** control: pick a profile and a goal, see the slice the router would expose and why.

## 10. Models and tiers (`/desk/models`)
Per profile: provider, capabilities (from AGENT-010 §3), decode ladder, exposure budget, context budget (advertised vs effective vs reserve), calibration (bytes/token with sample size), scorecard (AGENT-009 §4) with per-suite pass rates and the last five versions as sparklines, hardware admission state for local models (what the probe found, what it refused), **Run scorecard** and **Run canary** actions. Providers and keys live here too (moved from the `LlmCredentialForm` on the agent page), with the proxy's usage totals.

## 11. Monitor (`/desk/monitor`) — the UI of COMP-007
Strategies table: name, venue, environment, size, status (`nominal / warning / breach / scaled_down / halted`), the breaching metric, rolling 30/60/90-day Sharpe vs backtest, fill quality, signal frequency drift; a per-row **Scale down / Halt / Resume** with the two-step pattern for halt. The **kill switch** panel: state, scope (global / venue / strategy), who/what tripped it and why, the reset control (typed confirmation; the reset is logged). A **timeline** of monitor events. When the switch is tripped, the top-bar indicator and a persistent banner show it everywhere in the app.

## 12. Usage (`/desk/usage`)
Spend by day, by conversation, by model, by layer; cache hit rate over time; cost per finding and per gate advanced (the numbers AGENT-009 publishes); budget policies (per conversation defaults, monthly cap with a hard stop and a warning threshold); export CSV.

## 13. Notifications
In-app (toast only for cross-page events — Meridian forbids toasts for panel-scoped failures), browser notifications (opt-in) for: approval waiting, question waiting, run finished, run halted/fenced, kill switch tripped, control-limit breach. Email/webhook later. A **quiet hours** setting.

## 14. States (Meridian §3.22 applied)
- **Waiting on job**: the conversation shows the job card with progress; no spinner; the composer says "Sending will steer the running turn".
- **Waiting on you**: the approval/question card is pinned above the composer and the controls row highlights `a/x`.
- **Paused**: a banner with who paused and when; resume is one key.
- **Disconnected**: last known state stays, marked stale with age; the controls row disables Steer/Approve until reconnected (they need the event stream).
- **Degraded tier refusing**: the refusal card explains what the tier cannot do and offers escalation.
- **Halted (kill switch)**: every screen shows the banner; the monitor page is one click.
- **Empty desk**: the Start panel with three worked starters and "Import a strategy".

## 15. Accessibility and keyboard
WCAG AA on both themes (Meridian §5.3 contrast report stands). Every card is focusable; arrow keys move between cards, `enter` opens, `esc` closes the panel. Live regions announce: new approval, new question, run finished. Reduced motion respects the OS setting. All icon-only controls have labels and tooltips.

## 16. Real-time
One SSE stream per open conversation (`/api/agent/sessions/{id}/events` today; `agent_events` after AGENT-010 §5) delivering typed events; the client keeps an event cursor and reconnects with backoff, marking the UI stale meanwhile. Desk home subscribes to a multiplexed `/api/desk/events` (running runs, approvals, monitor). The right-panel State tab renders from the latest `state_snapshot` event, not from replaying cards.

## 17. APIs the desk needs (existing ✓, new ✗)

| Need | Endpoint |
|---|---|
| conversations, messages, cancel, archive | ✓ `/api/agent/conversations…` |
| runs, events stream, steer, inbox | ✓ `/api/agent/runs…`, `/api/agent/sessions/{id}/events`, `…/steer`, `…/inbox` |
| approvals list/answer | ✓ `/api/approvals`, `…/answer` |
| profiles, usage | ✓ `/api/agent/profiles`, `/api/agent/usage` |
| pause/resume, fork, escalate | ✗ `/api/agent/runs/{id}/pause|resume`, `/api/agent/conversations/{id}/fork`, `/api/agent/runs/{id}/escalate` |
| working state snapshots, trajectory replay | ✗ `/api/agent/runs/{id}/state?step=`, `/api/agent/runs/{id}/trajectory` (AGENT-010 §5) |
| dossiers | ✗ `/api/dossiers/{ticket}`, `…/verify/{claim}`, `…/comments` |
| memory browse/edit | ✗ `/api/memory/{findings|negative|profiles|preferences|playbooks}` (AGENT-007 §8-equivalent) |
| catalogue + exposure simulation | ✗ `/api/catalogue`, `/api/catalogue/exposure?profile=&goal=` (AGENT-006) |
| calibration + scorecards | ✗ `/api/models/{profile}/calibration`, `…/scorecard` (AGENT-009) |
| monitor + kill switch | ✗ `/api/monitor/strategies`, `/api/monitor/kill-switch` (COMP-007) |
| strategy editor | ✗ `/api/strategies/compile|preview|diff|moves` (AGENT-011) |
| desk multiplex stream | ✗ `/api/desk/events` |
| lifecycle: stage, registration, hypotheses, chain, yield | ✗ `/api/agent/runs/{id}/lifecycle` (AGENT-013 §8 `get_stage`'s struct), `…/preregistrations`, `…/yield` |
| proposals | ✗ `/api/proposals`, `…/{id}/{approve\|deny}` (AGENT-013 §7.5; the approval is a `decision` row) |
| research sessions (continuous) | ✗ `/api/research/sessions…` per AGENT-014 §10 (list/create/get, pause/resume/stop, agenda, charter, digests, SSE) |

## 18. Build order

| Phase | Deliverable |
|---|---|
| 1 | Desk shell + routes; conversation with card stream (all card types on today's events), controls row, composer with constraints and modes; right-panel State (from `plan`/`core_tool`/`usage` events until snapshots exist) and Evidence tabs; approvals inline + global |
| 2 | Desk home; budget meter and cost tab; job cards from `/api/jobs/events`; notifications; command palette; keyboard map; both themes and densities; states pass |
| 3 | Run detail + replay scrubber on `agent_events`; dossier reader; memory tab and browser (findings, negative results, preferences) |
| 4 | Catalogue page with exposure simulation; models page with calibration and scorecards; usage page |
| 5 | Monitor page and kill switch (COMP-007); strategy editor with preview (AGENT-011); playbooks; sharing and export |

## 19. Acceptance (per screen, in the Meridian §6 form)

1. A running research turn can be understood in under a minute from the State tab alone: goal kind, current step, evidence count, budget, and what it is waiting on are all visible without opening a card.
1a. Every element deriving from fetched content carries a provenance mark naming its source,
   untrusted text renders as inert text with no markup or link activation, and the mark
   survives dossier export — asserted with a fixture containing HTML and a link.
2. Every id shown in any card resolves on click or renders struck-through; a test injects an unknown id and asserts the strike-through.
3. Approve, deny, pause, stop and steer work from the keyboard while the agent is mid-call, on a frontier run and on a local run.
4. The replay scrubber reproduces the prompt the model saw at step N byte-for-byte from `agent_events`, with per-layer token sizes matching AGENT-005's accounting.
5. A dossier opened cold shows verdict, evidence table, unresolved questions and reviewer critique above the fold; a claim's **verify** re-runs and updates in place.
6. The kill-switch banner appears on every route within one event of the trip and cannot be dismissed while tripped.
7. Theme switch mid-run causes no reflow; both densities pass the contrast report; every icon-only control has a label.
8. At 375px width the conversation, composer, controls row and approval cards are fully usable.
9. A running Research turn shows mode, workflow, ring and stage in the header and State tab; the pre-registration card renders before any trial and its steer creates an amendment; a proposal card shows its envelope and tickets and approve/deny work from the keyboard; a Chat turn that would need a trial ends with a Research offer whose *Start* opens a run with the same question. *(Added 2026-09-14, AGENT-013.)*
10. Toggling Continuous swaps the question field for the charter form and back without losing typed constraints; a continuous session's state, next item and budget are readable from *Running now*; a veto on a queued agenda item prevents its claim and appears in the digest; a reorder writes a `user` priority event; every id in a digest resolves; a grep of the desk source finds no user-visible "Program". *(Added 2026-09-14, revised 2026-09-15, AGENT-014.)*

## 20. Non-goals

- A general chat client (no plugins, no arbitrary web browsing UI): the desk is for research with this platform.
- Editing the trial ledger or gate verdicts from the UI: humans approve, they do not rewrite evidence.
- Mobile-native apps; the responsive web desk is the mobile surface.

## 21. Open questions

- Whether the right panel's State tab should be the *centre* on narrow screens (state first, stream second); prototype both in phase 1.
- Whether comments on dossiers should be their own object or memory `user_preference` entries with a dossier scope; ship as comments, revisit when AGENT-007 phase 3 lands.
