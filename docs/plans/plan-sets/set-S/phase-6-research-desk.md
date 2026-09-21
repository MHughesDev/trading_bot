# Phase 6 — The Research Desk

**Completion: 25% (6 / 24 tasks), plus 18 `PARTIAL` — five markers were wrong low and one was wrong high; twelve of the endpoints the desk calls do not exist on the running platform.**

> **This file was wrong.** Until 2026-09-16 it recorded all 24 tasks as `NOT STARTED`. The
> Research Desk is in fact built and shipping: `pages/DeskPage.tsx` under 13 routes, 18
> components totalling about 5,300 lines, `/agent` and `/research` already redirecting into
> it, 57 tests passing, `tsc -p tsconfig.app.json` clean and `npm run build` green.
>
> **The missing half was the backend, and it was three specific endpoints.** Searched across
> all of `crates/api/src`, not just the router. Two were built 2026-09-16:
>
> * `/api/catalogue` and `/api/catalogue/exposure` — **served** (S-6.19). Building it exposed
>   the larger defect: the desk filters every tool surface on `tool.ring` and nothing on the
>   server assigned one, so that filter's behaviour rested on how `undefined` compared.
> * `/api/desk/digest` — **served** (S-6.24) for the four line kinds that have a source here,
>   with the other three declared uncovered rather than approximated.
> * `/api/research/sessions` — **not a missing endpoint.** It is the tail of S-11.1, which
>   needs the `crates/research-session` crate, the `research_sessions` / `session_events` /
>   `agenda_items` / `session_digests` tables and a `JobKind::ContinuousResearch`, and which
>   depends on phases 10, 2, 4 and 7. Listing it beside the other two made it look like a
>   handler somebody forgot. Building it ahead of its own plan would mean inventing the
>   session schema, and phase 11's declared migration range (0092–0096) is already taken —
>   the tree is at 0121, the same collision ADR-S3-08 records.
>
> **That list of three was itself undercounted, and the correction is the useful part.**
> It came from searching `crates/api/src` for the endpoints the audit went looking for. A
> *systematic* diff — every `/api/…` literal in `frontend/src/api/*.ts` against every one
> in `crates/api/src` — found **twelve**, four of which no audit had named: the State
> panel's by-step fetch (`/api/agent/runs/{id}/state`), Replay's `/trajectory`, phase 10's
> `/lifecycle`, and the run controls `desk.ts` already marks unbuilt in its own comment.
>
> A list maintained by looking is wrong by the amount nobody looked, so this is now a
> test: `crates/api/tests/the_desk_calls_routes_that_exist.rs` fails on any path the desk
> calls and nothing serves, with an `UNBUILT` allowlist naming the task that owns each
> forward reference. An unserved endpoint renders as an empty panel, indistinguishable
> from a panel with nothing in it — the failure looks exactly like the success, which is
> why it survived three passes.
>
> Two more tabs render against a store that was never created: S-6.10 and S-6.18 browse
> phase 4's memory, whose migrations are all absent.
>
> **The desk was not green when this audit started.** `npm run build` failed, because
> `SettingsPage.tsx` still imported two components from the deleted `components/agent/`. It
> went unnoticed because `npx tsc --noEmit` — the command used as this frontend's green light
> — runs against the root `tsconfig.json`, a solution file holding nothing but `references`.
> It checks no files and exits 0 regardless. The real command is
> `npx tsc -p tsconfig.app.json --noEmit`.
>
> S-6.22 is the one task that is genuinely `DONE`, and it is done as of this audit rather
> than before it.

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


### The third track: controls that reached nothing, and numbers nobody measured

**`Scale down` was a button with no `onClick`** — beside a live strategy, on the
operator's page. `POST /api/monitor/strategies/{id}/{action}` has been served
since COMP-007. All three of the route's words are wired now, and the client
refuses **here** exactly what the route refuses **there**, stated before the
request rather than as a 400 after it: `scale` needs `1..=100` (zero is not "off"
— that is `halt`, a different record), `halt` needs a reason, and `resume` needs
a reason plus the typed scope, compared **case-sensitively because
`ResetRequest::validate` is**. A refusal renders with `role="alert"`: a control
that swallows one reports failure as silence, which on this page reads as
success.

**A hosted profile was declared measured for being hosted.**
`measured={measured || !p.is_local}` contradicted the line three rows below it on
the same card. The badge is what a person reads at a glance. It is
`multi_step_evidence !== null` and nothing else now.

**The Spend table printed three zeros nobody measured.** `cost_usd` is
`COALESCE(SUM(...),0)` and there is no price table (OQ-16), so a run that burned
half a million tokens showed a cost of 0 — while the Cost tab six tabs away says
"not priced… this is not $0". `dollars_per_verdict` is `null` and `String(null)`
put the word **null** in the table. A *genuine* zero is now distinct from an
unpriced one, and the CSV exports the rendered values so the caveat travels with
the number.

**`searchedFor` is real, and it is not `pinned`.** The note existed all along:
`drive.rs` answers `search_tools` itself and pushes `Note::CoreTool`, already on
the array the Tools tab reads. `found: []` (the registry matched nothing) is kept
apart from `found: null` (the note did not parse) — and the empty case is the one
the harness's own comment says its worst behaviours came from, so labelling
`pinned` as `searchedFor` would have hidden exactly it.

**Frontend: 193 → 239 tests in 27 files.**

### The guard was weaker than this document claimed

`the_desk_calls_routes_that_exist` scanned **`frontend/src/api` only**, so a call
written directly in a component was invisible to it — while this plan said the
test "fails the build for it". Two such calls were live:
`Operations.tsx`'s strategy editor posts to `/api/strategies/compile` and
`/api/strategies/preview`, and the router serves neither.

The scan now covers all of `frontend/src`, `.ts` and `.tsx`, and both paths are
declared in `UNBUILT` with the task that owns them. The claim is true now; it was
not before.

### One spec understatement, implemented against the server rather than the spec

AGENT-012 §11 asks for "a per-row Scale down / Halt / Resume with the two-step
pattern **for halt**". On this platform **resume is a reset**: it refuses without
a reason, refuses without the typed scope, and is rate-limited. So resume needs a
*heavier* confirmation than halt, not a lighter one — and §11's own next
sentence agrees for the kill switch. The server's requirement was implemented and
§11's sentence was not read as licence to ship a one-click resume.


### The fourth track swept for the pattern, and found the two worst instances

Three tracks had each found the same shape by accident — an unknown state
rendered as the safe one. The fourth went looking for it on purpose, across
`components/desk/` and `pages/`, asking one question of every `?? 0`,
`?? false`, `=== true` and `data?.`: **what does this render when the request
failed, versus when the answer was genuinely empty?**

**The kill-switch banner rendered nothing at all on a failed read.**

```ts
export function killSwitchTripped(s: KillSwitchState | null | undefined): boolean {
  if (!s) return false
```

The banner appears on every route, so **its absence is the claim that trading is
fine**. A `/api/trading/status` that 500'd, dropped or was never reachable
produced exactly the same screen as a healthy platform. There is a third outcome
now, and it says so in those words: *"Trading state unknown — this is not the
same as trading being armed."* Silent only while the first read is in flight.

**`Reset (logged)` had never once reset anything.** The client posted
`/api/trading/resume` with **no body**; `reset_kill_switch` takes
`Json(body): Json<ResumeBody>` *by value*, so axum's extractor rejected every
click before the handler ran, into a dropped promise. `Halt all` had the kinder
signature (`Option<Json<KillBody>>`) and the same silence — a 403 from
`permits_monitor_control` reached nobody, and it logged the reason *"manual kill
from /api/trading/kill"* against a human's name. Both now refuse **here** what
the route refuses **there**, with `confirm_scope` compared case-sensitively
against `Scope::Global`'s own `Display`.

**And a third copy of an expression that had already been deleted once.**
`measured: p.multi_step_evidence !== null || !p.is_local` — removed from
`Models.tsx` by an earlier track — was still in `DeskHome.tsx` **and**
`Conversation.tsx`, so a hosted profile read as calibrated on two surfaces.
Removing one instance of a rule and leaving two is the same shape as the feature
registry that was "removed" while its table stayed behind. It is one shared
function now, beside the only thing that consumes it.

Six more, each with a test that would have failed before: the book said *"the
monitor is not built yet"* about a monitor that ships; "No capability profile is
configured" was printed from a request that may never have returned; a
conversation nobody could read showed `idle`, the calm word; a stream that
dropped before delivering a frame showed **"0s old"**; the proposals queue — which
licenses rings 1 and 2 — went **completely blank** on any non-404 failure; and
every provider read *not configured*, with **Connect** beside it, from a
credential list that never answered.

**Frontend: 239 → 287 tests in 32 files.** Nine more surfaces were checked and
found correct, and the checking is recorded beside the fixes — a sweep that only
reports what it changed cannot be reviewed.

### What this track did *not* do, and why it matters more than what it did

It reported **no task finished to `DONE`**, and named the reason: the two closest
candidates, S-6.3 and S-6.13, already meet their written acceptance line by line.
Changing them would have counted verification as a fix, which an earlier track
was flagged for.

That is the honest read of this phase now. **It is not short of frontend effort;
it is short of server fields.** Environment (live/paper) is not in
`list_strategies`, so §2.4's distinction cannot be drawn without inferring it
from a naming convention — which is how a paper strategy comes to be read as
live. Tier, current step, tokens and cache are not on
`GET /api/agent/conversations`, so four of §2.2's Running-now columns have no
source. Per-row Pause is in `UNBUILT`.

Until those exist, phase 6's remaining `PARTIAL`s cannot close from this side.


### The fields arrived, 2026-09-21 — and three of them had no home at all

The fourth desk track's conclusion was that this phase is short of **server
fields**, not frontend effort. A track went after them. What it found is worth
more than what it served.

**A strategy's live/paper environment has no declared home with a writer.**
Three places look like one and all three are empty: `mlops.paper_deployment`
(migration `0105`) is the *designed* home and **nothing in the repository ever
inserts into it**; `strategy_instances` is written only by a test; `accounts` is
unreachable because `0121` made `orders.account_id` nullable precisely so the
paper engine could work without a row. What *is* recorded is
`orders.execution_mode`, and `environment` is served from there — a recorded
column rather than a naming convention. It can only return `paper` or `null`
today, because a CHECK confines it.

**Charter amendments recorded *which version*, never *what changed*.** The
charter column is overwritten in place and the `charter_amended` event carried
only `{version}`. Serving a list of version numbers would have closed the ticket
without answering the question, so `amend_charter` now computes the field-level
diff **before** the overwrite. Amendments already in the log serve
`changed: null` — not `{}`, which would claim they changed nothing.

**`GET /api/desk/running` is new and had no caller** the moment it was written,
which is this repository's dominant shape; the track said so itself. It carries
a **`not_recorded: [{field, why}]`** array — the route naming what it cannot
know, so a reader is told rather than shown a zero.

**And it fixed a bug that would have switched the scorer off.** `reorder` marked
**every** id in the body with the `user` source, and a `user` item bypasses
scoring entirely — so one drag from a client that posts the whole board (which is
what a drag-and-drop client does) disabled the selection policy for the session:
no scores, no exploration draw, no decision rows, silently. The fix subtracts the
longest run **already in board order** before asking what moved earlier, so
dragging a card *down* prioritises nothing either — strictly narrower than the
obvious rule, in the safe direction.

**api lib: 351 → 370.**

### The fields got readers, 2026-09-21 — and the live database moved the null branches into the middle of the page

The three fields the previous track served now have callers, `/api/desk/running`
has one too, and the drag contract two tracks had broken between them is closed.
**361 frontend tests in 39 files** (floor was 331 in 36), `tsc -b` clean.

**The drag gap is the shape to remember.** Track A5 found that `reorder` marked
every id in the body `user`, which bypasses scoring; H1 fixed the route to
subtract the longest run already in board order. But the *client* had been
written against the old rule and sent only the ids it believed had moved — so
after the fix, one drag on a four-card board posted one id and the route
answered **400**. Neither track was wrong on its own. **A rule removed in one
place and left in two** is this session's second defect shape, behind
built-but-unreached, and it only shows up when someone runs both sides against
each other.

`prioritisedBy` is **deleted**, not left unused. An exported filter is one the
next caller reaches for, and the next caller would have re-opened the same hole.

**What the running platform said that the plan did not expect.** The track
probed unauthenticated (401 = mounted, 404 = not) and ran read-only SELECTs:

- **54 of 55 `agent_runs` rows have `prompt_tokens`, `cache_read_tokens` and
  `trials_spent` all NULL**; 23 have no `tier`; **30 of 55 have no
  `conversation_id`**. The null branches are not the edge case — they are the
  common case. A `?? 0` would have invented a number on nearly every row, and
  the run→conversation join §2.2 describes **misses for more than half of
  them**, which is why the no-link/no-Stop branch exists rather than a dead link.
- **No run is in an active state** (failed 23, fenced 14, completed 12, refused
  6), so `/api/desk/running` answers `runs: []` on this box.
- **`orders` is empty and `monitor.strategy_state` is empty**, so `environment`
  is `null` for every strategy — "no orders — environment unknown" is what would
  actually render, not `paper`.
- `session_digests`, `research_sessions` and `charter_amended` are all at zero
  rows. The digest card, the agenda board and the amendment list have no live
  data at all on this machine.

**Three in-code claims were stale against the running server** and were
corrected: that the digest route does not select `spend_unit` (it does), that no
route reads amendments back (one does now), and that `list_strategies` sends
three columns and nothing else.

One trap was left in place and labelled rather than removed: the unused
`RunningRow` interface in `api/desk.ts` is §2.2's *sketch* and carries
`usd_tenth_cents`, `cache_hit_pct` and `step` — three numbers this platform
cannot produce. It now says so in its own doc comment, so the next reader does
not wire a panel to it.

### One trap worth passing on

`the_desk_calls_routes_that_exist` counts **any** `/api/…` string literal in
`crates/api/src`, doc comments included. Writing `/api/agent/runs/{id}/state` in
prose *to explain that it is unbuilt* made it read as built and failed the suite.


---

### The guard was narrower than its claim, and the claim was narrower than its goal, 2026-09-21

**416 tests in 45 files** (floor was 395 in 44), `tsc -b` clean.

S-6.22's guard was widened **first, and watched to fail**, before a single file
was migrated — 216 files scanned instead of 63 plus five named ones. The exact
counts at that moment: `text-text-dim` 25, `bg-surface-2` 21, `text-text-muted`
20, `bg-pnl-up` 7, `bg-pnl-down` 6, `text-warning` 1 — **27 files in union**. The
audit said 28; the 28th is the guard itself, which names the aliases in order to
ban them.

**Then the criterion turned out to be narrower than its own goal.** With all five
class spellings clean, a grep for the *colour names* found **36 more usages
across 16 files** reaching the same deprecated custom properties through other
utility prefixes — `text-pnl-down`, `border-pnl-up/30`, `bg-text-dim`,
`bg-warning/10`. **The acceptance grep would have returned clean with the
aliases still in service.** They were migrated too, and then the six
declarations were deleted from `tailwind.theme.css`, which is what the task body
asked for all along. Four aliases stay because screens still reference them, and
the file says why: deleting a name something still uses is worse than the alias.

Every mapping was read off the theme file rather than guessed — each alias was
declared as `var(…)` of a token that already had a modern name, so every
migration is a rename with no colour change. **Nothing was left in the allow-list
for want of an equivalent.**

**Two narrowings, one lesson.** A guard whose scope is narrower than its claim
turns an open question into a settled one. A *criterion* whose scope is narrower
than its goal does the same thing one level up, and no amount of test discipline
catches it — only somebody checking what the rule was for.

**S-6.22 stays `PARTIAL`.** The alias clause is now met and exceeded, but two
clauses of the same task are still unbuilt: browser-notification opt-in with
quiet hours does not exist, and no `aria-live` region announces a new approval, a
new question or a finished run — only the kill-switch banner has one.

**`cost_usd`'s honest branch was unreachable in the only state this platform is
in.** Its "not priced (OQ-16)" path requires `tokens > 0`, and the empty
`llm_usage` table denies exactly that — so the live payload rendered
**`$0.0000`**, a billing claim, from a table with no rows. The new
`nothingRecordedInLlmUsage()` fires only when **all four** counters are zero: a
single zero beside a non-zero one is a real measurement and still prints `0`.
Five of the thirteen new tests fail without it.

> **OPEN QUESTION (recorded, not silently resolved).** S-6.22's acceptance says
> the grep must return nothing *"outside `compat.css`"*, but these aliases were
> **never in `compat.css`** — the same task body says they live in
> *"`tailwind.theme.css` lines 85-94"*, which is where they were, and
> `compat.css` contains no alias and never did. The intent was implemented
> (nothing outside the file that declares them, and now nothing anywhere) and
> the discrepancy is recorded in the guard's own header rather than resolved by
> allow-listing the wrong file.

### The digest stopped lying to the operator, same day

`kinds_not_covered` is **computed rather than constant**: a kind is named only
when *that request* tried its source and failed, and the reason quotes the
store's own error string. A constant cannot go stale if there is no constant.
`finding` and `refuted` now come from the memory store with its archived and
superseded filters; `monitor` comes from `monitor.events`, where `is_drill`
prefixes **`DRILL —`**, because a rehearsal reading as a halt sends an operator
to a position that was never at risk.

**The budget line is a four-way truth table, not a `SUM`.** `agent_runs` carries
`tokens_in`/`tokens_out` and **27 of 55 runs have them — 2,526,695 tokens in —**
while `llm_usage` has no rows at all. So on this deployment **volume is measured
and cost is not**: `TokensOnly` reports the real number and states that the
dollar figure is absent; `Unmeasured` emits **no line**, because that is an
absent measurement and not a spend of zero. The other **28 finished runs
recorded no token count at all**, which is a third fact again, and gets its own
sentence rather than being summed in as zero.

Two refusals worth keeping. `rust_decimal` was declined for the
`NUMERIC(38,18)` column — a 96-bit mantissa cannot hold what that column admits,
and a decode error caught with `.ok()` turns a recorded observation into a
silent absence; the value is cast to text in SQL instead. And the memory store's
archived/superseded filter had to be duplicated against that file's explicit
warning — paid for with **a CI test that reads both files and fails if either
half drops out of either**, rather than with the comment the store asks for.

**`/api/agent/usage` reads the wrong table**, which is the same defect one layer
down and is now its own track: the tokens are in `agent_runs`, and the route
selects `llm_usage`.


### The right panel, 2026-09-21 — two tabs were blaming the run for a missing route

**395 tests in 44 files** (floor was 361 in 39), `tsc -b` clean.

**The default tab has been empty on every conversation this platform has ever
had, and it said the run's fault.** `deskApi.state` calls
`/api/agent/runs/{id}/state`, which **is not mounted** — `routes/mod.rs`
registers `/api/agent/runs`, `…/{id}`, `…/{id}/messages` and `…/{id}/cancel`
and nothing else; the live platform answers **404** there where every mounted
route answers 401. `optional()` folded that 404 to `null`, and `null` was also
what "no snapshot yet" looked like — so State printed *"No working state yet —
it appears as soon as the run's first step lands"* forever, and Evidence printed
*"This run has produced no ids yet"* **over runs with 89 recorded tool calls**.
Both sentences are claims about the run; the truth was a claim about the desk.
`ServedRead<T>` / `served()` now sits beside `optional()` and the two tabs give
three distinct sentences (`not_served`, `no_run`, unknown).

**The state was not reconstructed from the card stream**, and the refusal is the
point: §3.5 forbids it, and a reconstruction is a second answer that can
disagree with the agent's own.

**A zero that was never a measurement.** `Usage::prompt_tokens()` sums three
counters that all default to `0`, and `0 ?? input_tokens` is `0`, not the
fallback — so a provider that reported no usage printed a confident **0** in the
provider column and summed those zeros into the cumulative. Fixed with a
`nonZero()` read, deliberately **not** applied to `output_tokens`, where zero is
a real outcome. The fix exposed a second bug in its own first cut: the cache
share must divide by the provider's own prompt total, never by the
`input_tokens` fallback — 200/800 reads as 25% where 200/1000 is the truth — and
an existing test caught it.

**A parser bug that presented as the run being illegible.** `search_tools` notes
are rendered strings and the database holds **two arrow spellings**: 4 rows with
ASCII `->` and 2 with `→` from an older build. The regex knew only the first,
so a third of recorded searches rendered as *"a search this panel could not
read"* — inverting the one distinction the section exists for, between
`found: []` (a search that matched nothing) and `found: null` (the desk
failing). **Any parser over a rendered note is a compatibility surface**: the
harness rewrites the format and the old rows stay.

**The pin button was removed rather than wired.** It was passed no handler, and
had one been passed there was nothing to call: no route records a per-run pin,
and `Driver::route` builds `pinned` from exactly three sources —
`discovered_tools`, the active plan step's named tool, and a three-hit search
over that step's goal text. A human message is none of them. The panel now names
those three levers instead of offering a control that does nothing.

**`KILL_SWITCH_EVENT` deleted, and the swap found the bug it was standing in
front of.** `Operations.tsx` invalidates the kill-switch query after its global
halt and **`DeskHome.tsx` did not** — so a halt from the book panel left the
chrome silent for the full three-second poll, on the one screen where that
banner is all the operator is looking for. `KILL_SWITCH_QUERY_KEY` replaces the
dead export and the false comment is rewritten: the guarantee is three seconds,
plus immediate for a halt this client caused.

**Three open questions, none implemented either way.** §3.5's *"pin a tool for
this run"* names a mechanism that does not exist (OQ below). §19.6's *"within one
event"* has **no channel at all** — nothing on this platform pushes kill-switch
state to a browser, `useDeskStream` is per-run, and a trip from the risk engine
or another operator arrives by poll and by nothing else; the criterion is unmet
and was being held up by a comment describing a mechanism that did not exist.
And §3.5's *"acceptance criteria with ticks"* has nothing to tick —
`WorkingState.acceptance` is `string[]` on both sides of the wire, so a tick
would be an invented measurement.

**Three more live facts the plan did not expect:**

- **Zero `selection` events, against 2,111 rows of every other kind.**
  `Note::Selected` is the only note carrying a per-record "why retrieved" clause
  — exactly what S-6.10's Memory tab needs — but `expose_pinned` ships
  `retrieval: false` as the default, so it is never emitted; and even under
  retrieval its lines are bare record ids with no `RecordKind`, so nothing would
  separate memory records from tool results. **S-6.10's problem was never the
  store**, which is served and browsable — it is that the per-run brief has no
  writer and its nearest candidate is structurally unusable.
- **All 428 `usage` rows predate the current emitter.** The live payload is
  `{step, latency_ms, input_tokens, output_tokens}` only; the emitter now also
  writes `prompt_tokens`, `est_prompt_tokens` and `cache_read_tokens`. So
  S-6.12's acceptance — harness estimate versus provider count — is
  **unobtainable on every run recorded so far**, and `latency_ms` is on the wire
  and rendered nowhere.
- **`record_finding` lands on the stream as `core_tool`**, which `KIND_MAP`
  routes to a tool card. A finding the agent recorded renders as a tool call,
  not as a Finding card with a confidence pill and evidence chips. §3.2's
  Finding card **has no live producer**.


### An audit that wrote no code, 2026-09-21 — six markers were wrong, in both directions

A track was sent to verify every status line in this file against the tree and
the running platform, and forbidden to edit anything. It is the cheapest track
of the session and it found more than most of the ones that built things.

**Five tasks were done and marked `PARTIAL`** — S-6.2, S-6.3, S-6.4, S-6.5,
S-6.13 — each promoted above with the specific clause that was checked. **One
task was marked `DONE` and fails its own acceptance by twenty-eight files**
(S-6.22). S-6.1 is almost certainly a sixth promotion; it is held back only
because its acceptance names a clean `npm run build` and a frontend track is
editing the tree as this is written. A build run against a half-finished edit
proves nothing, so the marker waits for one that means something.

**How S-6.22 passed.** Its guard walks the desk directory plus five named files
and asserts a claim about the whole repository. A test whose scope is narrower
than its claim is worse than no test, because it converts an open question into
a settled one. That is the lesson, not the twenty-eight files.

**Four status notes stated reasons that are false**, and they had propagated:
three separate notes said phase 4's memory migrations were absent. They are
applied — `0122`, `0124`, `0125` — `agent_memory.entry` exists with its full
CHECK set, and `/api/memory/{bucket}` serves all seven buckets. **The false
claim was not only in the plan: it is shipped to the operator in a response
body.** `/api/desk/digest` returns `kinds_not_covered` telling them *"findings
are not persisted in this deployment — the memory engine exists and its
migrations do not"*, and its second entry says the monitor tables *"record
current state, not state changes"* when `monitor.events` (migration `0075`)
carries `from_state`, `to_state`, `metric`, `observed_value` and `limit_value`
and is precisely a state-change log. A route built to prevent a silence from
being misread now misreads it out loud.

### What the running system said that the plan did not expect

- **The order gate is closed right now, and correctly.** No `tbot-monitor`
  container is running — `docker ps` shows postgres, nats, redis, clickhouse and
  nothing else, and the platform runs as a native binary. `monitor.lease` has 0
  rows, so `LeaseWatcher` holds the gate at `Missing` and `/api/orders` refuses
  every order with `monitor_unavailable`. The safety property works. **Nothing
  in either plan file records that the default local stack has no monitor in
  it**, which means every phase-7 claim about runtime behaviour has never been
  exercised on a developer machine.
- **All eleven `monitor.*` tables exist and all eleven are empty**, as are
  `strategy_instances`, `orders` and `fills`.
- **`llm_usage` has zero rows against 55 `agent_runs`.** Only the LLM proxy
  writes it and the desk driver does not route through the proxy, so
  `/api/agent/usage` returns zeros for input, output, cache read, cache write
  **and** cost. **The first version of this note said that made the Cost tab
  "nothing". That was wrong and the correction matters:** the right-panel Cost
  tab does not read that route at all — it folds the per-call `usage` notes off
  `/api/agent/runs/{id}/messages`, which is mounted and carries real numbers
  (`input_tokens: 11528` on a live row). The surface `llm_usage` actually
  damages is **`Operations.tsx` `usageRows()`**, whose default branch prints
  `input_tokens`, `output_tokens`, `cache_read` and `cache_write` as bare `0`.
  `cost_usd` and `cache_hit_rate` already have honest branches; those four do
  not. An all-zero read of that table means "not recorded by this table", not
  "nothing spent", and the frontend cannot tell the two apart from the numbers
  alone — so it needs a stated caveat, not an inferred one.
- **More than half the card taxonomy has never carried a row.** Of 2,111
  `agent_messages`: `status` 601, `usage` 428, `compaction` 202, `decision` 201,
  `exposure` 185, `tool_call`/`tool_result` 89 each, `policy` 89, `core_tool` 52,
  `validation` 45, `degradation` 32, `canary` 27, `plan` 26, `user` 25, `error`
  20 — and **zero** rows of `finding`, `negative_result`, `question`,
  `approval`, `gate`, `job`, `escalation`, `report` or `final`.
  `approval_requests` is 0 rows too. S-6.4 and S-6.5 are complete code over
  kinds no run has produced.
- **Not one stored message carries a provenance mark.** 0 of 2,111 rows have
  `provenance` or `evidence_class` in `content_json`, and `events.ts:129`
  defaults an absent mark to `'platform'` — **so every real card asserts
  platform provenance.** S-6.23's fence is tested, passing, and unreached by
  data, which is this repository's dominant defect shape in the one place it is
  least affordable.
- **`KILL_SWITCH_EVENT` has no dispatcher and no listener.** Exported at
  `KillSwitchBanner.tsx:16`, referenced nowhere else in `frontend/src`, while
  the banner's doc comment claims a live event refetches immediately so *"within
  one event"* holds. It is a three-second poll. "Within one event" is S-6.21's
  entire acceptance and a phase-7 exit criterion.
- **Twelve frontend-referenced API paths are unmounted on the running
  platform**, confirmed by probe (401 = mounted, 404 = not):
  `/api/agent/runs/{id}/{pause,resume,escalate,state,trajectory,lifecycle}`,
  `/api/strategies/{compile,preview}`, `/api/strategies/{id}`,
  `/api/automations/{id}/{action}`, `/api/automations/{id}/stages`,
  `/api/reports/{id}`. Every one is wrapped in `optional()`, **which folds a 404
  to `null`** — so nine desk controls are silent no-ops rather than errors.
- **The "Audited 2026-09-16" boilerplate repeated verbatim under 22 tasks is
  stale in every instance.** It claims 18 components, ~5,300 lines, 13 routes
  and 57 tests; the tree holds 20 components, 7,594 lines, 15 routes and 37 test
  files. Individual counts are off by up to 4× (`Operations.tsx` noted at 257,
  now 1,095). A line count copied under every task is a number nobody
  recomputes — the same failure the spec status lines had.


## Four desk tracks, 2026-09-21 — and what they found

**The desk's failure mode is a `null` literal with a phase number attached to
it.** `Conversation.tsx` handed six tabs `={null}` or `={[]}`, each explaining its
own emptiness by naming a Set S phase. That copy is what made the defect
invisible: an unwired panel and an unbuilt endpoint rendered identically, and the
panel's own explanation told the reader to stop looking. The catalogue had been
served for days behind exactly such a sentence, and `deskApi.sessions()` had
**zero callers anywhere in the app**.

The audit, and what each one turned out to be:

| prop | endpoint | served? | outcome |
|---|---|---|---|
| `AgendaTab items` | `…/{id}/agenda` | yes | wired; loading / did-not-answer / genuinely-empty are three states |
| `CharterTab charter` | `…/{id}` | yes | wired through a decoder — the wire shape is not the shape the tab assumed |
| `continuous` | same route | yes | wired; both tabs were unreachable on the only kind of session that has them |
| `CostTab calls` | none — **it was on the stream** | n/a | wired to the harness's own `usage` note |
| `ToolsTab exposed` | none — **it was on the stream** | n/a | wired to `Note::Exposure`, the exact note the prop documents |
| `MemoryTab items` | genuinely unbuilt | no | copy corrected: the *store* is browsable, the per-run **brief** has no writer |

**The most dangerous thing either track found was not in this list.**
`Operations.tsx` computed `tripped = kill.data?.kill_switch_active === true`, so
an endpoint that **did not answer** rendered a green *armed* badge — an unknown
safety state shown as the safe one, on the control that stops money moving. There
is now a third word, `state unknown`, and a test that pins it. The same file
rendered every live strategy row as two blanks and a dash, because it read
`name`/`status`/`breaching_metric` where the route sends
`strategy_id`/`state`/`scale_pct`; and its `scope` badge printed `global` from a
client-side default the route has never sent.

**Also fixed:** the budget meter printed `0 / 40,000` for a run that had burned
half a million tokens (`cap = remaining` makes `used` zero), and the trials bar
printed `4 / 4 · 90%+` for a run nothing had given a cap to. The away digest could
not tell "nothing was refuted" from "I cannot see refutations". *Nothing is
waiting on you* was asserted without looking, so a run suspended in
`AwaitingApproval` read as an evening with nothing to do.

**Frontend: 103 → 193 tests in 22 files, `tsc` clean.** Every new test asserts a
rejection rather than a happy path, and each would have failed before its change.

**What the desk still cannot show, and why** — each is a field or a route, not a
component: charter *amendments* are appended by `PUT …/{id}/charter` and read back
by nothing; per-layer prompt accounting has no emitter; there is no price table
(OQ-16), so Cost says "not priced… this is not $0"; `GET /api/agent/usage` keys on
an `agent_sessions` id and the desk's runs are `agent_runs`, so calling it with a
run id returns all-zeros — deliberately not wired; and there is no route from a
child conversation back to its session, so §3.1's session chip cannot be built.

### S-6.1 The desk shell — M
`PARTIAL` 2026-09-16 — `pages/DeskPage.tsx` (162 lines) with 13 routes, mounted at `/desk/*` in `App.tsx`, and `/agent`, `/research` and `/approvals` redirecting into it.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

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
`DONE` — promoted 2026-09-21 by audit, not by new work. All four acceptance
clauses hold and were checked individually: `useDeskStream.ts:125` sends
`Last-Event-ID` and `agent_sessions.rs:206` honours it; the events route answers
401 on the running platform (mounted); backoff reaches 15s; `stale` keeps the last
state with an age and is `null` when nothing ever arrived; `mergeCards` dedups on
`seq`. Superseded note: `PARTIAL` 2026-09-16 — `api/desk.ts` (582 lines) and `hooks/useDeskStream.ts` (158 lines). One stream, and the components read it rather than polling.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

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
`DONE` — promoted 2026-09-21 by audit. Both acceptance clauses are in
`CardStream.tsx`: `WINDOW = 25` mounts only the last 25 step groups, which is the
200-step jank clause, and `storageKey(conversationId)` persists the disclosure set
per conversation inside try/catch. Superseded note: `PARTIAL` 2026-09-16 — `CardStream.tsx` (116 lines).

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

Not a transcript: cards grouped by step under a step header ("Step 7 · sweep parameters · 2
tools · 41 s"), with raw events (exposure, compaction, validation, usage) folded behind a
`details` disclosure, off by default, on with `d`.

**Acceptance:** a 200-step run renders without jank; the disclosure state persists per
conversation.
**Traces:** AGENT-012 §3.2.

### S-6.4 Cards, part 1 — L
`DONE` — promoted 2026-09-21 by audit. All six card kinds are in
`KIND_MAP` (`lib/desk/events.ts`); `Cards.tsx:160` renders `'(n unknown)'` for a
null `n` rather than `0`; the tool card is prose with namespace, risk, duration and
handle, never a table. **But see the live note below: zero rows of the `finding`,
`question`, `approval`, `gate` or `final` kinds have ever been written.** The code
is complete over a taxonomy no run has produced. Superseded note: `PARTIAL` 2026-09-16 — `Cards.tsx` (548 lines) covers both card tasks.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

You · Plan (status glyphs, the tool each step names) · Tool (name, namespace, risk badge,
collapsed arguments, duration, the `ToolResult` summary as prose with linked ids, `n`/`se`
inline, handle) · Finding (confidence pill, evidence chips, "cited later") · Negative result
· Decision (what was chosen and why, with an override).

**Acceptance:** a tool card shows the summary, never a raw table; an `n`-less number renders
"(n unknown)".
**Traces:** AGENT-012 §3.2; §19.1.

### S-6.5 Cards, part 2 — L
`DONE` — promoted 2026-09-21 by audit. All nine kinds are mapped, and
`Cards.tsx:229` `IRREVERSIBLE = {SealedHoldout, Promotion, Tier3Memory}` drives the
two-step confirm, so the acceptance — that an irreversible action cannot be taken
in one click — is structurally unsatisfiable rather than merely untested. Same
caveat as S-6.4 about kinds with no rows. Superseded note: `PARTIAL` 2026-09-16 — `Cards.tsx` (548 lines), shared with S-6.4.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

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
`PARTIAL` 2026-09-16 — `ControlsRow.tsx` (174 lines), with the keyboard map exercised by `__tests__/controls-and-replay.test.tsx`.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

Pause · Resume · Stop · Steer · Approve/Deny · Escalate · Budget · Details · Replay, always
visible, never scrolled away. `space` pause/resume, `esc` stop (confirm), `a`/`x` approve or
deny the focused approval, `s` steer, `d` details, `r` replay, `/` composer, `?` shortcuts.

**Depends on:** S-2.12 for pause/steer/escalate to exist on every tier.
**Acceptance:** every control works from the keyboard **while the agent is mid-call**, on a
frontier run and on a local run.
**Traces:** AGENT-012 §3.4; §19.3.

### S-6.7 The composer — M
`PARTIAL` 2026-09-16 — `Composer.tsx` (247 lines).

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

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
`PARTIAL` 2026-09-16 — The State tab in `RightPanel.tsx` (690 lines), asserted by `__tests__/state-panel.test.tsx`. Fixed on 2026-09-16: `mode` and `kind` both render "research" and both rendered bare, so the panel showed two identical badges meaning different things. The mode is labelled now, as the ring already was.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

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
`PARTIAL` 2026-09-16 — The Evidence tab in `RightPanel.tsx`.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

Every id the run produced, grouped, with in-place viewers: experiment summary, study surface
(the plateau/spike plot — `charts/Primitives.tsx` has the parts), gate ledger, trade list
with the concentration bar, bars chart, data-quality results.

**Acceptance:** every id resolves on click; an unresolvable id renders struck-through with
"not found" — a test injects an unknown id and asserts the strike-through.
**Traces:** AGENT-012 §3.5, §6.3, principle 9; §19.2.

### S-6.10 Right panel — Memory — M
`PARTIAL` 2026-09-16 — The Memory tab in `RightPanel.tsx`. **The stated reason was
false and is corrected here:** phase 4's migrations are *applied* — `0122`, `0124`
and `0125` are all live, `agent_memory.entry` exists with its full CHECK set, and
`/api/memory/{bucket}` is mounted. The real defect is the one this repository keeps
producing: the tab has **no caller with data**. `Conversation.tsx:489` is literally
`<MemoryTab items={null} />`. The store is also empty (0 rows), which is a
different fact again and must not be rendered as the same one.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

What the brief contained, each item marked *cited* when the run used it, with "why
retrieved" on hover, and **add to memory** / **forget**.

**Depends on:** S-4.5.
**Acceptance:** every brief item shows why it was retrieved and whether the run cited it;
**forget** removes it from the next brief.
**Traces:** AGENT-012 §3.5; AGENT-007 §4.

### S-6.11 Right panel — Tools — M
`PARTIAL` 2026-09-16 — The Tools tab in `RightPanel.tsx`, with `lib/desk/rings.ts` deciding what a mode may show.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

What this tier can currently see, what it searched for, the whole catalogue with search,
tool notes on click, and pin-a-tool for this run.

**Depends on:** S-2.4.
**Acceptance:** the panel's exposed set matches the harness's `Note::Exposure` for that step
exactly; pinning a tool makes it available on the next step.
**Traces:** AGENT-012 §3.5.

### S-6.12 Right panel — Cost — M
`PARTIAL` 2026-09-16 — The Cost tab in `RightPanel.tsx`. `/api/agent/usage` is served; the dollar figures behind it are **OQ-16** — the platform has no price table, so the tab can show tokens and not money.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

Tokens by layer (charter / schemas / body / output) per call, cache hit as a sparkline,
cumulative dollars, and the calibrated bytes-per-token beside the harness estimate and the
provider's count.

**Depends on:** S-2.8, S-4.12.
**Acceptance:** per-call layer totals sum to the provider's reported prompt tokens within the
calibrated error; the cache sparkline is per call, not per run.
**Traces:** AGENT-012 §3.5, principle 11.

### S-6.13 The shared components — M
`DONE` — promoted 2026-09-21 by audit. All eight named components are
exported from `Shared.tsx` and imported into `DesignSystemPage.tsx` under its
"Research desk" section, and `/design` is a public route. Superseded note:
`PARTIAL` 2026-09-16 — `Shared.tsx` (375 lines).

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

Tier badge (with measured/unmeasured), budget meter (four bars, cap, 90% line, projected
end, `warn` at 75% / `neg` at 90%, always with the text value), evidence chip, confidence
pill, risk badge, envelope banner, job progress, step header. All added to `/design`.

**Acceptance:** every one appears on the `/design` page; none uses colour alone (Meridian
principle 3).
**Traces:** AGENT-012 §6.

---

## The other surfaces

### S-6.14 Desk home — L
`PARTIAL` 2026-09-16 — `DeskHome.tsx` (263 lines). Fixed on 2026-09-16: it cast `Conversation` to `Record<string, unknown>` for stringly-typed access on a shape the compiler already knew, which is what made the frontend fail to typecheck.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

Needs you (approvals, questions, escalations — proposals and stalled continuous sessions join
it in S-10.15 / S-11.13) · Running now (tier, current step, what it waits on, cost, pause/stop
per row; continuous sessions join it in S-11.13) · Recent conclusions · The book · Tier
status · Start (with the *Chat / Research* mode control, the Continuous toggle, and three starters).

**Acceptance:** "Needs you" is empty-stated with the last decision made, never a bare "No
data" (Meridian §3.22); the Start panel's mode control sends the mode with the turn.
**Traces:** AGENT-012 §2 (amended 2026-09-14).

### S-6.15 Approvals, inline and global — M
`PARTIAL` 2026-09-16 — `ApprovalsInbox.tsx` (198 lines) plus the approval card in `Cards.tsx`; `/api/approvals` is served. Fixed on 2026-09-16: an irreversible approval had nowhere to say **why**. The two-step confirm worked, but `onApprove`'s `note` was declared in the type and passed by nobody — and `spend_holdout_look` refuses without twenty characters of reason on the other side.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

Reuse `components/workspace/ApprovalsInbox.tsx`, which already handles eight approval kinds
with evidence and timeouts. Keep `/approvals` as an alias of `/desk/approvals`.

**Acceptance:** an approval answered inline disappears from the global inbox in the same event;
`/approvals` still resolves.
**Traces:** AGENT-012 §1, §3.2.

### S-6.16 Run detail and the replay scrubber — L
`PARTIAL` 2026-09-16 — `RunDetail.tsx` (172 lines) with the replay scrubber, exercised by `__tests__/controls-and-replay.test.tsx`.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

A step strip; the working state **as it was** at that step; the model call's rendered prompt
sections with their token sizes; toggles for prompt / reply / diff. **Save as case** writes
the trajectory into `evals/`.

**Depends on:** S-2.5, S-4.8.
**Acceptance:** the scrubber reproduces the prompt the model saw at step N **byte-for-byte**
from `agent_events`, with per-layer token sizes matching AGENT-005's accounting.
**Traces:** AGENT-012 §4; §19.4.

### S-6.17 The dossier reader — L
`PARTIAL` 2026-09-16 — `Dossier.tsx` (268 lines), including the Markdown export and the untrusted-chain check.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

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
`PARTIAL` 2026-09-16 — `MemoryBrowser.tsx`. **The stated reason was false and is
corrected here:** the store exists, `routes/memory.rs` serves all seven buckets and
refuses `playbooks` by name, and this component genuinely calls it. The real gap is
that `agent_memory.entry` has **0 rows**, so it browses an empty store rather than
an absent one — and those must render differently.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

Findings · Negative results (with "retry now" opening a conversation with the brief) ·
Instrument profiles · Preferences · Playbooks · Signal registry. A conflicts filter shows
disagreeing pairs side by side with a "resolve" that starts a diagnose conversation.

**Depends on:** Phase 4 memory half.
**Acceptance:** a conflicting pair renders side by side and neither is hidden by ranking; every
row opens its provenance (run, tool, data window).
**Traces:** AGENT-012 §7.

### S-6.19 The catalogue page — M
`DONE` 2026-09-16 — `crates/api/src/routes/desk_catalogue.rs` serves `/api/catalogue` and `/api/catalogue/exposure`.

**The listing was the small half.** `lib/desk/rings.ts` filters every tool surface in the desk on `tool.ring`, refusing to offer a trial-spending tool inside a Chat turn — and **nothing on the server ever assigned a ring**. That is not a missing badge: `RING_INDEX[undefined] <= 0` is `false`, so the filter's real behaviour was decided by which way a comparison against `undefined` happened to fall. `ToolSpec::ring()` now derives it from the risk and envelope a tool already declares, so it cannot disagree with the gate a human sees, and four tests cover both directions plus the reachability of the ring-2 branch.

Derived rather than listed, deliberately: an array of tool names beside the catalogue is the shape `internal_agent_tools` had until S-1.3, and a miss here is worse than silent — a paper-reaching tool absent from the list would report ring 0 and become clickable in a Chat turn.

**Acceptance met.** Simulate-exposure returns the slice the router would choose because it *is* the router: `route_goal` moved into `ToolRegistry` and `drive.rs` calls it, so one implementation serves both. Two copies could only have made that true by inspection, on the page that exists to let someone argue with the router's choices. Reasons are given for the tools that did not make the cut as well, which is the half worth arguing with.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

Namespaces as columns, tools as cards (tier visibility, risk, trial flag, output budget,
notes), workflows as sequences with decision points, and **simulate exposure**: pick a
profile and a goal, see the slice the router would expose and why.

**Depends on:** S-1.3, S-2.4.
**Acceptance:** simulate-exposure returns the same slice the router would choose for that
profile and goal, and names the reason each tool was included.
**Traces:** AGENT-012 §9.

### S-6.20 Models and tiers — M
`PARTIAL` 2026-09-16 — `Models.tsx` (221 lines). It is where AI providers moved to, and as of 2026-09-16 the Settings page links here instead of importing a deleted component.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

Per profile: capabilities, decode ladder, exposure and context budgets, calibration with
sample size, scorecard with per-suite pass rates and five-version sparklines, hardware
admission state, **Run scorecard** and **Run canary**. Providers and keys move here from
`LlmCredentialForm` on the agent page.

**Depends on:** S-4.11, S-4.13.
**Acceptance:** a profile with no calibration row renders `unmeasured` rather than a default;
providers and keys work here and the old form is gone.
**Traces:** AGENT-012 §10; AGENT-009 §4.

### S-6.21 Usage, monitor, strategy editor — L
`PARTIAL` 2026-09-16 — `Operations.tsx` (257 lines) and `KillSwitchBanner.tsx`; `/api/agent/usage` and `/api/monitor/strategies` are both served.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

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
`PARTIAL` — **downgraded from `DONE` on 2026-09-21. It fails its own
acceptance by twenty-eight files.** The criterion is *"a grep for `bg-surface-2`,
`text-text-muted`, `text-text-dim`, `bg-pnl-down`, `text-warning` returns nothing
outside `compat.css`"*. That grep returns **28 files** — mlops 7, workbench 6,
automations 5, pages 4 and more. The task passed because its guard,
`__tests__/no-deprecated-aliases.test.ts`, walks only the desk directory plus five
named files, so it asserts a repository-wide claim over a subdirectory. Two further
clauses are unmet: browser-notification opt-in and quiet hours do not exist, and no
`aria-live` region announces a new approval, a new question or a finished run —
only the kill-switch banner has one. Superseded note: `DONE` 2026-09-16 — `components/agent/` and `components/workspace/` are gone, nothing imports them, and no desk file uses a back-compat alias — all three asserted by `__tests__/no-deprecated-aliases.test.ts`, which passes.

It did not pass before 2026-09-16, and both halves of the failure were real. `SettingsPage.tsx` still imported two components from the deleted `components/agent/`, which **broke `npm run build`** — proven with a scratch file, exit 2, TS2307. Service tokens had been lost rather than moved, so `ApiAccessCard` was rebuilt under `components/settings/`; the MCP server cannot start without a token that card is the only way to mint. The alias scan also read `desk.css`'s own header — which names the forbidden tokens in order to forbid them — as a violation, and now strips comments the way `static_invariants.rs` does.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

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
`PARTIAL` 2026-09-16 — The provenance fence is asserted by `__tests__/honesty.test.tsx`: untrusted text renders inert, the card names its source, and the mark survives the Markdown export. All passing.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

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
`PARTIAL` 2026-09-16 — `crates/api/src/routes/desk_digest.rs` serves `/api/desk/digest`.

Four of the seven line kinds have a source in this deployment and are served: `concluded` from `agent_runs.summary` (the run's own conclusion, falling back to its error and only then to its goal), `approval_defaulted` from timed-out `approval_requests` **naming what defaulted**, `budget` from `llm_usage` summed over the gap, and `waiting` for what is still pending. The under-an-hour rule is implemented and tested, and it returns `absent: true` rather than an empty list — the two render differently and mean different things.

**Three kinds are declared uncovered rather than approximated**, which is what keeps this `PARTIAL`: `finding` and `refuted` need phase 4's memory migrations, which do not exist; `monitor` needs a state-*change* log, and `monitor_lease` / `kill_switch` / `monitor_limits` record current state, so a change inferred by diffing against a guess at the old state would report changes that did not happen. The response carries `kinds_not_covered` with a reason for each, because an empty digest and a blind one look identical and the operator reading it is deciding whether to stop looking. A test asserts every kind in the frontend's union is in exactly one of the two lists.

Audited 2026-09-16 against the source. The desk is built: 18 components and `pages/DeskPage.tsx` under 13 routes, ~5,300 lines, `/agent` and `/research` already redirecting to it, 57 tests passing and `tsc -p tsconfig.app.json` clean.

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
