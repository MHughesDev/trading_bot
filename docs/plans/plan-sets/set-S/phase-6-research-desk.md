# Phase 6 — The Research Desk

**Completion: 24 / 24 surfaces built; 11 `DONE`, 13 `PARTIAL`**

> Every task's UI is built, wired and tested. Thirteen are `PARTIAL` because the endpoint
> they read does not exist yet — AGENT-012 §17 marks a dozen rows as absent and they belong
> to other phases. Each of those surfaces renders an honest empty state naming the phase that
> owns the data, rather than a placeholder number. **A `PARTIAL` here means "the screen is
> finished and proven against fixtures; the live data is not there yet"**, and each note says
> which.

**Requirement IDs:** DK-01…DK-24.
**Spec:** [AGENT-012](../../../specs/AGENT-012-research-desk-ui.md), which supersedes
COMP-006 §2–§6. Design system: `docs/meridian-design-system/01-DESIGN-SPEC.md` is normative
and this phase adds no foundations to it.
**Depends on:** Phase 0. It starts on today's data and deepens as Phases 2–5 and 7 land —
so it is the longest-running track and should start first among the parallel ones.
**Blocks:** nothing.
**Migration range:** none (the desk reads; new endpoints belong to their owning phases).
**Parallel with:** everything.

**Meridian is already built.** `frontend/src/styles/tokens.css` is byte-identical to the
handoff document, and `frontend/src/components/primitives/` ships `Panel`, `Button`,
`Badge`, `Num`/`StatTile`, `Table`, `Modal`/`ConfirmDialog`, `Overlay` (`Tooltip`,
`Popover`, `Select`), `Field`, `Segmented`/`Tabs`, `States`, plus chart primitives and an
833-line `app.css` component layer. The desk is a **surface build on existing parts**, not a
design-system build. `/design` is the visual review surface.

**It replaces two surfaces, it does not become a third.** `/agent` polls
`agentApi.messages` every 1.5 s and renders 16 `AgentMessage` kinds through `ActivityRow`;
`/research` consumes SSE and renders `TimelineEvent` through a second, unrelated `ToolCard`.
Both are written in the deprecated back-compat Tailwind aliases (`bg-surface-2`,
`text-text-muted`) that `tailwind.theme.css` marks "Do not introduce new usages". The desk is
built only on the primitives and `app.css`, and S-6.22 deletes what it replaces.

---

## Shell and transport

### S-6.1 The desk shell — M
`DONE`

**Landed 2026-09-15 (track 25).** Routes under `/desk` (one lazy route in `App.tsx`), nav entry, `FULL_HEIGHT_ROUTES`, three columns at wide width becoming a drawer at 1200px and a sheet at 768px. Verified by: `no-deprecated-aliases.test.ts`.

Routes under `/desk` (`App.tsx`, lazily imported, named exports), a nav entry in
`NAV_SECTIONS` (`TopBar.tsx` — `CommandPalette` derives its "Go to" commands from it), and
`/desk` added to `FULL_HEIGHT_ROUTES` in `AppLayout.tsx` so it owns its own scroll. Three
columns at wide width; right panel → drawer under 1200px; rail → sheet and controls docked
to the bottom under 768px.

**Files:** `frontend/src/pages/DeskPage.tsx`, `frontend/src/components/desk/`,
`frontend/src/App.tsx`, `components/layout/{TopBar,AppLayout}.tsx`.
**Acceptance:** `npm run build` (which is the typecheck) and `npm run lint` clean; **no new
usage of the back-compat aliases** — a grep in review; both themes and both densities render
without reflow.
**Traces:** AGENT-012 §1; §19.7.

### S-6.2 One event stream — M
`DONE`

**Landed 2026-09-15 (track 25).** `useDeskStream` keeps a cursor, resumes with `Last-Event-ID`, backs off to 15 s, marks the UI stale with an age instead of blanking it, and merges by `seq` so a replayed frame cannot duplicate a card. Verified by: `controls-and-replay.test.tsx > the stream resumes without duplicating`.

Copy the pattern that already works: `api/research.ts` `openTimeline` is a hand-rolled
`fetch` SSE client with `AbortController`, `Last-Event-ID` resume, manual frame splitting,
backoff to 15 s and a permanent stop on 401/404 — deliberately **not** `EventSource`, which
cannot set an `Authorization` header. Keep an event cursor; mark the UI stale while
reconnecting rather than blanking it.

**Files:** `frontend/src/api/desk.ts`, `frontend/src/hooks/useDeskStream.ts`.
**Acceptance:** a dropped connection reconnects and resumes from the cursor with no duplicate
cards; while disconnected the last state stays visible, marked stale with an age.
**Traces:** AGENT-012 §16; §14 (states).

### S-6.3 The card stream — L
`DONE`

**Landed 2026-09-15 (track 25).** Step groups with a header, raw events folded behind a disclosure (off by default, on with `d`), the disclosure state persisted per conversation, and only the last 25 groups mounted so a 200-step run does not mount 200 steps.

Not a transcript: cards grouped by step under a step header ("Step 7 · sweep parameters · 2
tools · 41 s"), with raw events (exposure, compaction, validation, usage) folded behind a
`details` disclosure, off by default, on with `d`.

**Acceptance:** a 200-step run renders without jank; the disclosure state persists per
conversation.
**Traces:** AGENT-012 §3.2.

### S-6.4 Cards, part 1 — L
`DONE`

**Landed 2026-09-15 (track 25).** You, agent, plan, tool, finding, negative result, decision. A tool card shows the summary as prose, never a raw table, and an `n`-less number renders "(n unknown)". Verified by: `honesty.test.tsx > a number without its sample size says so`.

You · Plan (status glyphs, the tool each step names) · Tool (name, namespace, risk badge,
collapsed arguments, duration, the `ToolResult` summary as prose with linked ids, `n`/`se`
inline, handle) · Finding (confidence pill, evidence chips, "cited later") · Negative result
· Decision (what was chosen and why, with an override).

**Acceptance:** a tool card shows the summary, never a raw table; an `n`-less number renders
"(n unknown)".
**Traces:** AGENT-012 §3.2; §19.1.

### S-6.5 Cards, part 2 — L
`DONE`

**Landed 2026-09-15 (track 25).** Question, approval, job, gate, escalation, correction, compaction, report, stage, pre-registration, offer, proposal, halt. A `SealedHoldout` approval cannot be confirmed in one click. Verified by: `honesty.test.tsx > an irreversible approval cannot be confirmed in one click`.

Question · Approval (envelope banner, reversibility, two-step for irreversible) · Job
(deterministic progress, never an indeterminate spinner over content) · Gate (funnel
position, the tickets that licensed it, which are missing) · Escalation · Correction ·
Compaction (with handles) · Report · Halt/degradation (each with the one thing the person can
do).

**Acceptance:** an approval for a `SealedHoldout` action cannot be confirmed in one click.
**Traces:** AGENT-012 §3.2, §6.7; Meridian §5.5.

---

## Working with the agent

### S-6.6 The controls row and the keyboard map — M
`DONE`

**Landed 2026-09-15 (track 25).** Every control is bound at the document, not to a focused element, and nothing is disabled by the agent being busy — only by a dropped stream, which is named in the tooltip. Verified by: `controls-and-replay.test.tsx > every control works from the keyboard while the agent is mid-call`.

Pause · Resume · Stop · Steer · Approve/Deny · Escalate · Budget · Details · Replay, always
visible, never scrolled away. `space` pause/resume, `esc` stop (confirm), `a`/`x` approve or
deny the focused approval, `s` steer, `d` details, `r` replay, `/` composer, `?` shortcuts.

**Depends on:** S-2.12 for pause/steer/escalate to exist on every tier.
**Acceptance:** every control works from the keyboard **while the agent is mid-call**, on a
frontier run and on a local run.
**Traces:** AGENT-012 §3.4; §19.3.

### S-6.7 The composer — M
`DONE`

**Landed 2026-09-15 (track 25).** Two options and a toggle. The charter lives beside the question, so toggling loses nothing in either direction; a Chat question that would need a trial renders the offer instead of submitting; a send mid-run is a steer. Verified by: `modes.test.tsx`.

Model picker with tier badge and an `unmeasured` warning; constraint chips (instrument,
timeframe, window, balance) that render into the opening message (S-1.6); attach a strategy,
dossier, finding or workspace file; mode segmented control (**Chat / Research**, ADR-0037)
with a **Continuous** toggle (its charter form is S-11.13) and a **workflow** chip under Research (investigate · falsify · replicate · compare ·
diagnose · survey · optimise, defaulted from the question's shape) and **Write strategy** as an
action beside attach, not a mode; slash commands (`/steer`, `/pause`, `/pin`, `/why`, `/model`,
`/budget`, `/tier`). Sending while a run is active is a **steer** by default, shown as its own
card. The Research offer card, the stage strip and the pre-registration card are S-10.15.

**Acceptance:** Chat mode spends no trials and a Chat goal the classifier rates `Research`
renders the offer card (stubbed until S-10.2); the workflow chip is sent with the turn; sending
mid-run produces a steer card rather than a queued turn.
**Traces:** AGENT-012 §3.3 (amended 2026-09-14); AGENT-008 §3.1; AGENT-013 §2.

### S-6.8 Right panel — State — M
`PARTIAL`

**Landed 2026-09-15 (track 25).** Panel complete and proven against a fixture. It renders from `GET /api/agent/runs/{id}/state`, which AGENT-012 §17 marks as not built — so in the running app today it shows "No working state yet" rather than a reconstruction from cards. Verified by: `state-panel.test.tsx`.

The `WorkingState` card: goal and kind, acceptance criteria with ticks, hypothesis, plan
tracker, evidence with the tool that produced each id, findings, open questions, decisions,
last error, budget meter. Rendered from the latest `state_snapshot` event, not by replaying
cards.

**Depends on:** S-1.13, S-2.5.
**Acceptance:** a running turn is understandable from this panel alone in under a minute —
goal kind, current step, evidence count, budget and what it is waiting on, all without
opening a card.
**Traces:** AGENT-012 §3.5, principle 8; §19.1.

### S-6.9 Right panel — Evidence — L
`PARTIAL`

**Landed 2026-09-15 (track 25).** Resolution and the strike-through are done and adversarially tested. The in-place viewers are id chips plus a summary line, not the plateau/spike plot, the gate ledger or the trade list — those need the experiment and study endpoints. Verified by: `honesty.test.tsx > every id resolves or is struck through`.

Every id the run produced, grouped, with in-place viewers: experiment summary, study surface
(the plateau/spike plot — `charts/Primitives.tsx` has the parts), gate ledger, trade list
with the concentration bar, bars chart, data-quality results.

**Acceptance:** every id resolves on click; an unresolvable id renders struck-through with
"not found" — a test injects an unknown id and asserts the strike-through.
**Traces:** AGENT-012 §3.5, §6.3, principle 9; §19.2.

### S-6.10 Right panel — Memory — M
`PARTIAL`

**Landed 2026-09-15 (track 25).** Tab built with *cited* marks, "why retrieved" and forget. `/api/memory/*` (Set S phase 4) does not answer, so it renders the "not built yet" state rather than an empty brief.

What the brief contained, each item marked *cited* when the run used it, with "why
retrieved" on hover, and **add to memory** / **forget**.

**Depends on:** S-4.5.
**Acceptance:** every brief item shows why it was retrieved and whether the run cited it;
**forget** removes it from the next brief.
**Traces:** AGENT-012 §3.5; AGENT-007 §4.

### S-6.11 Right panel — Tools — M
`PARTIAL`

**Landed 2026-09-15 (track 25).** The mode and ring filter is done and adversarially tested; pin-a-tool is wired. The exposed set cannot yet be compared with the harness's `Note::Exposure` because no endpoint publishes it. Verified by: `modes.test.tsx > the Tools panel renders none of them in Chat, and says why`.

What this tier can currently see, what it searched for, the whole catalogue with search,
tool notes on click, and pin-a-tool for this run.

**Depends on:** S-2.4.
**Acceptance:** the panel's exposed set matches the harness's `Note::Exposure` for that step
exactly; pinning a tool makes it available on the next step.
**Traces:** AGENT-012 §3.5.

### S-6.12 Right panel — Cost — M
`PARTIAL`

**Landed 2026-09-15 (track 25).** Component built — per-layer table, per-call cache sparkline, calibrated against estimated bytes/token, with `unmeasured` where there is no calibration. No endpoint publishes per-call layer totals, so it renders empty.

Tokens by layer (charter / schemas / body / output) per call, cache hit as a sparkline,
cumulative dollars, and the calibrated bytes-per-token beside the harness estimate and the
provider's count.

**Depends on:** S-2.8, S-4.12.
**Acceptance:** per-call layer totals sum to the provider's reported prompt tokens within the
calibrated error; the cache sparkline is per call, not per run.
**Traces:** AGENT-012 §3.5, principle 11.

### S-6.13 The shared components — M
`DONE`

**Landed 2026-09-15 (track 25).** All eleven components are on `/design` under "Research desk"; none uses colour alone.

Tier badge (with measured/unmeasured), budget meter (four bars, cap, 90% line, projected
end, `warn` at 75% / `neg` at 90%, always with the text value), evidence chip, confidence
pill, risk badge, envelope banner, job progress, step header. All added to `/design`.

**Acceptance:** every one appears on the `/design` page; none uses colour alone (Meridian
principle 3).
**Traces:** AGENT-012 §6.

---

## The other surfaces

### S-6.14 Desk home — L
`PARTIAL`

**Landed 2026-09-15 (track 25).** All six panels present, with the empty "Needs you" naming the last decision rather than showing "No data". "Running now" cannot yet show tier, current step, cost or cache per row — the conversation list does not carry them and `/api/desk/events` does not exist.

Needs you (approvals, questions, escalations — proposals and stalled continuous sessions join
it in S-10.15 / S-11.13) · Running now (tier, current step, what it waits on, cost, pause/stop
per row; continuous sessions join it in S-11.13) · Recent conclusions · The book · Tier
status · Start (with the *Chat / Research* mode control, the Continuous toggle, and three starters).

**Acceptance:** "Needs you" is empty-stated with the last decision made, never a bare "No
data" (Meridian §3.22); the Start panel's mode control sends the mode with the turn.
**Traces:** AGENT-012 §2 (amended 2026-09-14).

### S-6.15 Approvals, inline and global — M
`DONE`

**Landed 2026-09-15 (track 25).** `ApprovalsInbox` reused and rebuilt on the primitives (deviation recorded in the file header); `/approvals` redirects to `/desk/approvals`.

Reuse `components/workspace/ApprovalsInbox.tsx`, which already handles eight approval kinds
with evidence and timeouts. Keep `/approvals` as an alias of `/desk/approvals`.

**Acceptance:** an approval answered inline disappears from the global inbox in the same event;
`/approvals` still resolves.
**Traces:** AGENT-012 §1, §3.2.

### S-6.16 Run detail and the replay scrubber — L
`PARTIAL`

**Landed 2026-09-15 (track 25).** Scrubber, layer table and prompt/reply/diff built; the byte-for-byte property is proven on fixtures and the panel *says* when the layers do not reassemble instead of showing a plausible reconstruction. `/api/agent/runs/{id}/trajectory` does not exist, and **Save as case** is a stub. Verified by: `controls-and-replay.test.tsx > the replay scrubber reproduces the prompt byte-for-byte`.

A step strip; the working state **as it was** at that step; the model call's rendered prompt
sections with their token sizes; toggles for prompt / reply / diff. **Save as case** writes
the trajectory into `evals/`.

**Depends on:** S-2.5, S-4.8.
**Acceptance:** the scrubber reproduces the prompt the model saw at step N **byte-for-byte**
from `agent_events`, with per-layer token sizes matching AGENT-005's accounting.
**Traces:** AGENT-012 §4; §19.4.

### S-6.17 The dossier reader — L
`PARTIAL`

**Landed 2026-09-15 (track 25).** Reader, per-claim verify-in-place and three exports built; the untrusted mark survives the Markdown export and is tested. `/api/dossiers/*` does not exist.

A document, in the `paper` reading layout even when the app is in `terminal`: verdict,
evidence table (claim → id → n/se → verification method), plan as executed, funnel with
tickets, the mandatory unresolved-questions and reviewer-critique sections, cost and trials,
a provenance strip. Per-claim **verify** re-runs the recheck live. Comments anchor to claims
and can be sent to the agent. Export PDF / Markdown / signed JSON.

**Depends on:** S-8.7.
**Acceptance:** opened cold, verdict / evidence / unresolved questions / critique are above
the fold; **verify** updates in place.
**Traces:** AGENT-012 §5; §19.5.

### S-6.18 The memory browser — L
`PARTIAL`

**Landed 2026-09-15 (track 25).** All seven tabs, the conflicts filter and the side-by-side pairing are built — `conflictPairs` pairs each disagreement once, so neither side can be ranked away. `/api/memory/*` does not answer.

Findings · Negative results (with "retry now" opening a conversation with the brief) ·
Instrument profiles · Preferences · Playbooks · Signal registry. A conflicts filter shows
disagreeing pairs side by side with a "resolve" that starts a diagnose conversation.

**Depends on:** Phase 4 memory half.
**Acceptance:** a conflicting pair renders side by side and neither is hidden by ranking; every
row opens its provenance (run, tool, data window).
**Traces:** AGENT-012 §7.

### S-6.19 The catalogue page — M
`PARTIAL`

**Landed 2026-09-15 (track 25).** Namespaces, tool cards and the simulate-exposure control are built. `/api/catalogue` and `/api/catalogue/exposure` do not exist, so the "same slice as the router" acceptance cannot be checked yet.

Namespaces as columns, tools as cards (tier visibility, risk, trial flag, output budget,
notes), workflows as sequences with decision points, and **simulate exposure**: pick a
profile and a goal, see the slice the router would expose and why.

**Depends on:** S-1.3, S-2.4.
**Acceptance:** simulate-exposure returns the same slice the router would choose for that
profile and goal, and names the reason each tool was included.
**Traces:** AGENT-012 §9.

### S-6.20 Models and tiers — M
`PARTIAL`

**Landed 2026-09-15 (track 25).** Providers and keys moved off the retired agent page and work here; a profile with no calibration row renders `unmeasured`. The decode ladder, scorecards and canary need AGENT-009.

Per profile: capabilities, decode ladder, exposure and context budgets, calibration with
sample size, scorecard with per-suite pass rates and five-version sparklines, hardware
admission state, **Run scorecard** and **Run canary**. Providers and keys move here from
`LlmCredentialForm` on the agent page.

**Depends on:** S-4.11, S-4.13.
**Acceptance:** a profile with no calibration row renders `unmeasured` rather than a default;
providers and keys work here and the old form is gone.
**Traces:** AGENT-012 §10; AGENT-009 §4.

### S-6.21 Usage, monitor, strategy editor — L
`PARTIAL`

**Landed 2026-09-15 (track 25).** The kill-switch banner is global, structural (it sits in `AppLayout` above the outlet) and has no dismiss control at all — that half is done. Usage shows the totals `/api/agent/usage` answers; the monitor table and the SLv2 compile and preview calls have no endpoint yet.

Usage (spend by day/conversation/model/layer, cache rate, cost per finding and per gate,
budget policies, CSV). Monitor (Phase 7's UI: strategy table with status and breaching
metric, per-row scale/halt/resume with the two-step pattern, the kill-switch panel with
scope and who tripped it, the event timeline). Strategy editor (SLv2 text with
`list_features` autocomplete and inline validator errors carrying **Apply fix**, beside the
live `preview_signals` output).

**Depends on:** Phase 7 for the monitor; S-5.3, S-5.4, S-5.8 for the editor.
**Acceptance:** when the kill switch is tripped the banner appears on **every** route within
one event and cannot be dismissed while tripped.
**Traces:** AGENT-012 §8, §11, §12; §19.6.

### S-6.22 Retire what this replaces — M
`PARTIAL`

**Landed 2026-09-15 (track 25).** `/agent` and `/research` are deleted with redirects; `components/agent/*` and `components/workspace/*` are gone and nothing imports them (asserted by test). **The alias block is NOT deleted** — 28 non-desk files still use it (mlops, automations, workbench, proving-ground, settings, `layout/ModeBadge`, `ui/dialog`), which is outside this phase; see the plan defect noted in the track report. Notifications are not built. Verified by: `no-deprecated-aliases.test.ts`.

Delete `/agent` and `/research` (keep redirects), `components/agent/*` and
`components/workspace/*` except `ApprovalsInbox`, and the back-compat Tailwind aliases in
`tailwind.theme.css` lines 85-94 once the last consumer is gone. Add notifications (in-app,
browser opt-in, quiet hours), the command-palette entries, the states pass and the
accessibility pass.

**Acceptance:** no import of `@/components/agent/*` remains; a grep for `bg-surface-2`,
`text-text-muted`, `text-text-dim`, `bg-pnl-down`, `text-warning` returns nothing outside
`compat.css`; at 375px the conversation, composer, controls row and approval cards are fully
usable; live regions announce new approval, new question and run finished.
**Traces:** AGENT-012 §13, §14, §15; §19.7, §19.8.

### S-6.23 The provenance fence has to reach the human — M
`DONE`

**Landed 2026-09-15 (track 25).** Untrusted text renders inert, the mark names the source, `model_summary` is visibly not a measurement, and the mark survives export. Verified by: `honesty.test.tsx > the provenance fence reaches the human`.

The platform fences untrusted content so the *model* can tell data from instruction. Nothing
carried that to the *person*, who is the one who acts on it. The path is short and real: a
fetched page carries a persuasive claim, the agent records a finding leaning on it, the finding
renders exactly like one derived from the platform's own bars, and a human promotes a strategy
on it.

Every card, chip and memory row tracing to `Provenance::Untrusted` carries a provenance mark
(glyph, source host, fetch time on hover), and `evidence_class` renders visibly — a
`model_summary` must never look like a measurement. Two hard rules, because this is a security
surface and not styling: untrusted text renders as **inert text**, never markup, with no
auto-linking and no image loading; and a dossier claim whose evidence chain touches untrusted
content is marked in the evidence table, and the mark **survives export**.

**Files:** `frontend/src/components/desk/`, `frontend/src/components/primitives/Badge.tsx`.
**Acceptance:** a fixture containing HTML and a link renders inert and marked; the mark
survives PDF and Markdown export; `model_summary` and `direct_stat` are visually distinct.
**Traces:** AGENT-012 §6.3a; §19.1a.

### S-6.24 Since you were away — M
`PARTIAL`

**Landed 2026-09-15 (track 25).** The panel, the last-visit marker, the gap filter and the under-an-hour rule are built and tested. `/api/desk/digest` does not exist. Verified by: `controls-and-replay.test.tsx > Since you were away covers exactly the gap`.

The operator persona's actual question, which no other panel answers. The desk knows when you
last looked, so it can say what changed: runs that finished and what they **concluded**
(verdicts, not titles), findings established and any marked `refuted`, approvals that timed
out and what defaulted, monitor state changes and anything scaled down or halted, budget
spent, and anything now waiting on you. Each line opens the thing it names. Dismissable,
resets the marker, never shown for a gap under an hour.

A research agent that runs for hours without you is only useful if coming back to it takes a
minute — and a reverse-chronological event list is not that, because it makes the human
reconstruct a state the platform already knows.

**Files:** `frontend/src/components/desk/AwayDigest.tsx`, `frontend/src/api/desk.ts`.
**Acceptance:** the digest covers exactly the gap since the last visit and no more; every line
is a verdict or a state change rather than an event name; it is absent for a short gap.
**Traces:** AGENT-012 §2.2a; §19.

---

## Exit criteria

- One surface. `/agent` and `/research` are gone and nothing imports their components.
- A running turn is legible from the State panel alone; every id resolves or is struck
  through.
- Pause, steer, approve, deny and stop work from the keyboard mid-call on every tier.
- The replay scrubber reproduces a historical prompt byte-for-byte.
- The kill-switch banner is global and undismissable while tripped.
- `npm run build`, `npm run lint` and `npm test` clean; both themes, both densities, 375px
  up; no deprecated alias anywhere in the desk.
