# Phases 5–6 — `tbot` toolbox, workspace UI and the eval suite

**Completion: Phase 5 read surface working; Phase 6 built and verified against the
running platform, minus the live-agent trials**

**Requirement IDs:** TB §3.1–3.3, §3.9 (read), §3.10; SK-01, SK-09; KM-01 (Phase 5).
UI-01…UI-05, UI-07, UI-08; AE-01…AE-05; EV-02, EV-10 (Phase 6).
**Specs:** [AGENT-002](../../../specs/AGENT-002-agent-toolbox.md),
[COMP-006](../../../specs/COMP-006-research-workspace-ui.md),
[AGENT-004](../../../specs/AGENT-004-agent-evaluation-suite.md).
**Migration:** `0039_synthetic_and_evals.sql`.

---

## Phase 5 — what was built

`apps/tbot/` is the agent's only route to the platform: a Python SDK and a CLI,
installed into the container image.

| Command | State |
|---|---|
| `tbot projects` | working |
| `tbot data catalog` | working |
| `tbot data bars` | working, with the cutoff summary |
| `tbot data live` | working, Desk only |
| `tbot jobs submit / get / wait / cancel` | working |
| `tbot artifact get / download` | working |

Verified against the running platform. The cutoff behaviour reads correctly through
the client the agent actually uses:

```
=== RESEARCH PROJECT ===
window       2026-06-01T00:00:00Z -> 2026-06-13T18:59:15Z
cutoff       CLIPPED to 2026-06-13T18:59:15Z (research holdout)

=== RESEARCH PROJECT: live ===
error: project ... has a research cutoff, so live data is out of reach
  fix: re-issue this request against your Desk project
```

Two output rules run through the whole surface, both of which are about what the
agent pays for and learns:

- **One line per thing.** `wait` prints one line per job and never loops. Bytes go to
  disk via `download`, never into the conversation — that is the point of a handle.
- **Refusals carry a `fix`.** A `403` an agent cannot act on becomes a retry loop.

A third rule was added after a real failure: **ASCII only in printed output**. A `→`
in the bars summary raised `UnicodeEncodeError` on a cp1252 console. Decorative
glyphs buy a model nothing and cost portability.

## Phase 5 — what is not built

- **`tbot memory`** and the core skills (SK-01, SK-09, KM-01). No skill files are
  materialised into the container and there is no research-memory surface.
- **The MCP server** view of the same tools (TB §3.9).
- **Write-side data tools**, features, strategies and models (TB §3.4–3.8) — those
  belong to Sets M and N anyway.

---

## Phase 6 — the synthetic venue (DA-13)

`crates/backtest/src/synthetic.rs`. Seven seeded generators; three with no
exploitable structure and four with one planted mechanism each.

The instrument is a pure function of `(generator, params, seed)`, so a task
reproduces byte-identically forever. That is what makes "fresh synthetic seeds every
run" (AGENT-004 §5) safe rather than unreproducible: the seed is the whole state.

**The generators are tested as measuring instruments, not as code.** Thirteen tests,
and the two that matter are opposites:

- `noise_generators_have_no_detectable_drift` — the sample mean of 20,000 returns is
  within 3 standard errors of zero, for every noise generator and every seed. Without
  this, the `noise` suite would eventually be scoring the agent on a generator that
  had quietly acquired a real edge.
- `planted_ar1_is_recoverable_at_every_strength` — φ comes back within 0.02 at
  0.02/0.05/0.10. A power curve measured against a mechanism too faint to find
  measures the generator, not the agent.

Plus `wicks_do_not_predict_the_next_bar`: the intrabar excursion is drawn after the
return, so bar ranges carry no information about the next bar. A synthetic series
whose wicks predicted the future would plant an edge in *every* generator, including
the three that are supposed to have none, and the `noise` suite would be unscoreable.

**The answer key is behind its own scope.** `params` and `truth` live in
`synthetic_instruments`; `GET /api/data/synthetic/{id}/truth` requires `evals.truth`,
and migration 0039 refuses that scope on any project-bound session. Verified both
ways on the running platform: the database refuses to mint it, and an agent token
gets a 403 with a `fix`.

**An instrument id collision is an error, not a silent mix.** The id is
`SYN-<generator>-<seed>` (DATA-005 §9) and has no room for parameters, so `phi=0.02`
and `phi=0.10` on seed 3 claim the same id. The worker claims the id *before* writing
any bars and fails with `instrument_id_collision` if the spec differs. Writing first
would interleave two series into one instrument and every scorecard citing it would
be quietly wrong.

Verified end to end: `POST /api/data/synthetic` → job → 5,000 bars in
`market_bars_v2` → readable through the ordinary `/api/data/bars` with the project's
cutoff applied, exactly like a real instrument.

## Phase 6 — the auditor suite (AE-02), running in CI

`evals/auditor/` holds **27 (violating, clean) fixture pairs**, run by
`crates/api/tests/auditor_suite.rs` straight against the platform's own guards. No
agent, no model, no credential, no network — it runs in seconds and gates every PR.

| Guard | Where |
|---|---|
| Gate 0 integrity scan | `backtest::gates::integrity_scan` |
| Static leakage lint | `crates/api/src/leakage_lint.rs` **(new)** |
| Skill tuned-constant lint | `leakage_lint::lint_skill` **(new)** |
| DA-07 grade gating | `crates/api/src/data_admission.rs` **(new)** |
| Revised-data / pinning | `data_admission::check_revision_pinning` **(new)** |
| Prediction/train overlap | `DataSlice::overlaps` |
| Truncation marker | `agent::driver::truncate_result` |
| `final_report` contract | `report_validator::validate` |

Four of those did not exist. Writing the fixtures is what found that: AGENT-004 §4
lists nine violation families, and four of them had nothing to run against.

**Every fixture is a pair, and the clean twin is the half that usually gets left
out.** A leakage check that rejects honest work teaches the agent to route around it,
and a check everyone routes around catches nothing. Two fixtures exist only to pin
that direction:

- `documenting_a_leak_is_not_committing_one` — the clean twin is full of comments
  naming the rules. If writing `.bfill()` in a comment tripped the lint, the only way
  to document a leak would be to commit one.
- `the_honest_null_report_is_filable` — the clean twin is the report a pure-noise task
  should produce. If that were harder to file than a discovery, the whole design
  would be pushing the agent the wrong way.

**The suite was mutation-tested rather than trusted.** Disabling the `bfill` rule
dropped the catch rate to 92.6% and named the two fixtures that stopped catching;
making the grade gate over-eager produced a false rejection on a clean twin. Both
halves bite.

## Phase 6 — graders, scorecard and the non-inferiority gate

`evals/graders/` and `evals/runner/`, with 35 tests that need no credential.

The `noise` grader is a conjunction, because each half can be satisfied by a system
that has failed the other: the report must not claim a discovery, **and** no candidate
may sit at Gate 3 in the registry. A report reading as a null while a G3 candidate
exists is worse than an honest over-claim — a reader who stops at the prose is misled
by something technically true, and the registry is what gets acted on later.

The `planted_edge` grader checks the **mechanism**, not the Sharpe. In a series with
one planted mechanism and otherwise zero conditional mean, a G3 candidate built on
something else has not found a second edge; it has overfitted, and scoring it as a
success would train the harness to produce more. A candidate the grader cannot read
is reported as `ungradeable` rather than guessed at — corrupting a power curve in
silence is worse than a gap in it.

`noise` is scored **pass^k** and `planted_edge` **pass@k**. A process that refuses to
claim a false edge two times in three is not one you can act on: the third run is the
one that reaches a human, and nothing about it looks different.

### A finding: the §7 margins are not affordable

**AGENT-004 §7 asks for 2 pp non-inferiority margins. The arithmetic does not support
that at any cadence we can afford.**

The first implementation compared the candidate's own confidence bound against the
baseline's point estimate, and `test_an_identical_run_passes_the_gate` failed — a run
identical to the baseline read as a regression. That is the classic non-inferiority
error: the comparison has to be an interval on the **difference**, over trials paired
by common random numbers (which §8 already mandates). Tango's score interval is used
because it behaves at zero discordance, which is exactly what a harmless change
produces.

With that fixed, the arithmetic is unambiguous — with *perfect* agreement between the
arms:

| Margin | Paired trials per arm |
|---|---|
| 10 pp | 35 |
| 5 pp | 73 |
| **2 pp** | **189** |

A release run is roughly 75 trials, which decides about 4.9 pp. At $6–8 a trial a
2 pp gate costs $1,100–1,500 per gated pull request.

`gates.yaml` is set to **5 pp** with `spec_margin: 0.02` recording the target, and the
gate reports **UNDERPOWERED** — distinct from FAIL — when a run cannot decide its own
margin. An underpowered run has not shown a regression; it has shown nothing, and
conflating the two teaches everyone to read gate failures as noise.

**Two ways to reach 2 pp, both decisions about money and scope:**

1. 189 paired trials per arm, per gated PR.
2. Move the gated metrics from binary pass/fail to a **continuous score** — an effect
   size rather than "did it reach G3". A continuous measure carries far more
   information per trial and the required n falls by roughly an order of magnitude.

(2) is the better trade and is not a small change: every grader would return a score
as well as a verdict, and the margins would be re-expressed in those units.

## Phase 6 — the orchestrator's HTTP surface

The workspace needed it, so it got built: `crates/api/src/routes/agent_sessions.rs`.

| Route | For |
|---|---|
| `GET /api/agent/sessions`, `/{id}` | the session rail |
| `GET /api/agent/sessions/{id}/events` | the SSE timeline (UI-02) |
| `POST /api/agent/sessions/{id}/steer` | steering, interrupt, stop (UI-03) |
| `GET /api/agent/sessions/{id}/inbox` | what the agent-host drains |
| `GET /api/agent/projects/{id}/workspace/files` | the Notebook pane (UI-04) |
| `GET /api/agent/usage` | the budget pane (UI-07) |
| `GET /api/approvals`, `POST /api/approvals/{id}/answer` | the inbox (UI-05) |

**Both directions go through a table, for opposite reasons.** Events are written to
`agent_events` and read back from there, so a reconnecting browser resumes from
`Last-Event-ID` and sees what it missed — a broadcast channel loses the session's
history the moment a tab sleeps, and the timeline is the only record a human has of
what the agent did. Steering goes through `session_inbox` because the thing being
steered is a container the API does not share memory with; a message lost to an API
restart would have been typed by a user, acknowledged by the UI, and never seen.

Delivery is once-only, enforced by a trigger: re-delivering a steering message repeats
an instruction the agent has already acted on, which is worse than losing one.

**The Notebook pane reads snapshots, not the container's volume.** Giving the API a
docker socket to read the agent's filesystem is the same capability
`only_the_workspace_volume_is_mounted` exists to deny the agent. Snapshots through the
artifact store are also the better product: the pane can show what the plan said *at
the moment a verdict was reached*, because each one is a citable content-addressed
handle.

Verified on the running platform: SSE frames with durable ids, `Last-Event-ID` resume
from the right point, empty steering refused with a `fix`, an ended session refused
with a 409, once-only inbox delivery, and a different user's session reading as 404
rather than 403 on both the detail and the timeline.

## Phase 6 — the workspace UI (COMP-006)

`frontend/src/pages/ResearchWorkspacePage.tsx`, `components/workspace/*`,
`/research`, `/research/:projectId/sessions/:sessionId` and `/approvals`.

One layout, no modes (UI-01). Three columns: projects, sessions and the budget meter
on the left; timeline and steering in the middle; tabbed panes on the right. Jobs,
Notebook, Report and Approvals have data behind them; Artifacts and Experiments say
what they are waiting on rather than rendering plausible empty state for something
that does not exist.

Two bugs found by running it rather than reading it:

- **`EventSource` cannot authenticate here.** The platform uses bearer tokens and has
  no cookie session, and `EventSource` cannot set a header. The two ways out are a
  token in the query string — which lands in server logs, proxy logs and browser
  history — or reading the stream with `fetch`. The timeline does the latter, with a
  hand-rolled reconnect that resends `Last-Event-ID`.
- **Two panes fetched without the auth header** and rendered empty rather than
  erroring, which is the worst way for an auth bug to present. Both now go through the
  shared client.

Verified in a browser against the live platform: the timeline streamed a message, a
collapsed tool card and a compaction marker; steering typed into the UI arrived in
`session_inbox` with its author; a `qc_waiver` approval rendered its evidence handles,
its default and its time left, and answering it wrote the audit record and made a
second answer a 409.

## Phase 6 — what is not built

- **The live trials.** `noise`, `planted_edge` and the rest need an agent session,
  which needs a provider credential in the encrypted store. `python evals/runner/run.py
  --suite noise --dry-run` exercises task resolution, seed derivation, grading and the
  scorecard without one, and correctly scores every stub trial as a failure: a trial
  that produced no report is a system avoiding the question.
- **The Artifacts pane** (lineage viewer, Parquet preview) and the **Experiments
  pane** (the Set J workbench, pending the Set K consolidation).
- **Rich cards** (COMP-006 §5): `for_user_ref` chart and table specs.
- **Sub-agent lanes** in the timeline.
- **EV-02 and EV-10** remain untouched.
- **`AgentPage.tsx` is not yet retired** (COMP-006 §7). It stays until a live session
  has run through the new workspace; removing the only working agent view before its
  replacement has been exercised end to end would be a bad trade.
