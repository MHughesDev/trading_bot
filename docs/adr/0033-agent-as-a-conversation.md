# ADR-0033: The agent is a conversation, not a form

**Status:** Accepted
**Date:** 2026-09-12
**Amends:** ADR-0024 (agent runtime), ADR-0031 (harness conformance), ADR-0032 (local tier)
**Relates to:** D-04, D-05, D-18

---

## Context

The agent's entry point was a form: provider, model, goal, instrument, timeframe, max
iterations, time budget, **Start run**. Every field except the goal asked the user to
decide something before any work had happened — which is exactly when nobody knows the
answer.

- **Which instrument?** The answer is often "whichever one has coverage", and finding
  that out is the first thing the agent does.
- **How many iterations?** A sweep that converges in three and one that needs thirty
  are indistinguishable in advance.
- **How long?** A wall clock cannot tell a stuck session from one waiting on a
  forty-minute backtest, so it killed both.

Worse, a run was a single shot. There was no way to say "now try it on ETH" without
starting again from nothing, and no place for the agent to keep notes between attempts.

## Decision

### 1. One input: a message

The user prompts. The agent chooses the instrument, the timeframe, how many backtests
to run and when it is finished — all through tools it already had. D-05 said the
platform is agent-first and the human UI is a viewer over the same APIs; a form that
pre-constrained the agent was the opposite of that.

### 2. Conversations hold turns; a turn is still a run

`agent_conversations` is new; a turn is an `agent_runs` row with a `conversation_id`.
Keeping the run as the unit of agent work means the timeline, the approvals, the typed
outcome (ADR-0032) and the trace all keep working unchanged.

Each turn is given the **conclusions** of the earlier turns, not their transcripts.
Replaying every prior tool call would refill the window with work already done and
summarised; the detail is still in the agent's own folder if it wants it back.

One turn at a time per conversation. Two agents on one thread would interleave their
tool calls and their notes in one workspace, and the transcript would stop being a
sequence anyone could read.

### 3. No wall clock, no iteration cap

The agent runs until it calls `finish_task`.

**This is a deliberate removal of a safety net, and it is worth being explicit about
what is left.** The stop conditions that remain are the ones that mean something:

| Condition | What it catches |
|---|---|
| The user presses Stop | anything |
| `Degradation::Stuck` | the same failure surviving its retry budget |
| `Degradation::ConstraintIgnored` | the backend breaking a decoding guarantee |
| `Degradation::OutputTruncated` | replies that will not fit the output budget |

What is gone is the timer, and a timer only ever caught *slowness* — it fired on
healthy long work as readily as on a runaway. A loop that is genuinely broken hits
stuck-detection in a handful of steps; a loop that is merely slow now finishes.

The cost is real: a pathological session can spend tokens until someone notices. That
is a cost the user accepted explicitly, and the mitigation is the always-visible Stop
button plus the fact that spend is on the screen while it happens.

### 4. Compaction triggers at 95% of the window, not on every assembly

`context::COMPACTION_TRIGGER = 0.95`. Below it, context passes through whole.

This was a bug, found by reading a live trace: the per-section sub-budgets are *shares
of the whole window*, so applying them unconditionally truncated prompts that were
nowhere near full — **3,474 tokens cut to 420 against a 6,000-token budget**, because
the tool schemas exceeded the Tools section's 20% share while the prompt as a whole
used under half the window. The model was reading a truncated prompt for no reason.

Compaction is a response to pressure. With no pressure it is just deletion.

Deduplication is the exception and still runs unconditionally: removing a
byte-identical copy loses nothing, so there is no reason to wait for pressure.

The frontier driver's transcript elision now derives its threshold from the same
constant and the model's own profile, rather than a fixed 400,000 characters that
happened to be about right for one model.

### 5. Every conversation gets a folder

`agent-workspaces/agents/<conversation_id>/`, with the five areas from
`harness::workspace` and four `fs` tools over them: `write_file`, `read_file`,
`list_files`, `delete_file`.

Per conversation, not per run: an agent that loses its notes between turns is an agent
with no memory of its own work.

Containment is the existing lexical resolver — `..` is consumed against a stack and a
path that pops past the root is refused **before** it touches the filesystem, so a
symlink the agent planted is never followed. `delete_file` is `Risk::Destructive` and
therefore asks at every tier, even though the blast radius is the agent's own notes;
"it was only my own file" is a judgement the policy engine should not have to make.

`fs` calls are routed by the **driver**, not the loop — the loop stays sans-IO and
knows nothing about filesystems.

### 6. The conversation titles itself

One cheap constrained model call on the first prompt. A fallback cut from the prompt
is written **synchronously first**, because generating the real title is a model call
and on a cold local model that is nearly three minutes, measured — and a sidebar row
that says nothing for three minutes is a row the user cannot find.

Only turn 0 titles a conversation. A later turn re-titling the thread would move a row
the user had learned to find.

### 7. The UI follows the coding agents

Researched against Claude Code, Cursor and Codex, and against the 2026 agent-UX
literature. What was adopted:

- **A live step list, not a spinner.** One study put session abandonment at **3×**
  without mid-run visibility, at identical output quality.
- **Tool calls as collapsed cards** — name, summarised arguments, status icon,
  expandable to full input and output.
- **Activity is subordinate to the answer.** Traces are dimmed, monospace and one line
  tall; the result is full-size prose. A timeline where every entry shouts is one
  nobody reads.
- **Always-visible kill switch** while anything is running.
- **Reconnect, not restart.** The sidebar's running dot and the transcript are both
  queries against Postgres, so returning to a conversation after closing the tab is the
  same code path as opening it the first time.

## Consequences

- `POST /api/agent/runs` is gone. `GET` stays so historical runs remain inspectable.
- `agent_runs.max_iterations` and `wallclock_budget_secs` are nullable and unwritten;
  old rows keep their values.
- The UI polls rather than streams. The agent is a detached task writing to a table, so
  "what happened" is a query — and the reconnect case, which is the one that has to be
  right, is not a second code path.
- A `degraded` tier still refuses a research conversation, because research is
  multi-step work (D-18). That is not a regression; it is the tier doing its job.

## Alternatives considered

**Keep the form and add a chat on top.** Rejected: two entry points with different
capabilities is two things to keep in step, and the form's fields would still be
pre-constraining the agent whenever they were used.

**Stream over SSE instead of polling.** Deferred, not rejected. It would be faster to
first token, but the transcript is already durable and the reconnect path has to exist
either way — so streaming is an optimisation over a correct base rather than the base.

**Keep a generous wall clock (say 24h) rather than none.** Rejected on the user's
explicit instruction. Worth recording that the argument for it is weak anyway: a
24-hour timer would not have caught any failure mode that stuck-detection does not
already catch sooner.

**Give each *run* a folder rather than each conversation.** Rejected: the second turn
could not read what the first turn wrote down, which is most of the value of having a
folder at all.
