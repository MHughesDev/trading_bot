# BS-007 / 02 — What the agent is and how it behaves

Status: draft proposal. Positions here are recommendations for discussion, not decisions.

---

## 1. One sentence

A quant research agent that can answer any question about an instrument from the
platform's data, and can run a long, honest research session that designs, tests, models
and explains a trading algorithm for that instrument. Every number it reports comes from a
platform tool, and every performance claim comes from Set J.

## 2. Jobs to be done

| # | Job | Example request | Time scale | Primary tools (see 03) | Output |
|---|---|---|---|---|---|
| J1 | **Ask**: answer a question about an instrument or market | "How volatile has ETH been this month vs last year, and is it trending?" | seconds–2 min | market-analysis, live snapshot, knowledge | Short answer + numbers with n/SE + a chart for the user |
| J2 | **Investigate**: test a specific idea quickly | "Does BTC mean-revert after a 3σ hourly drop?" | 2–15 min | `event_study`, `describe_returns`, nulls | Conditional distribution vs null, with an evidence verdict and trial count |
| J3 | **Build**: create an algorithm for an instrument | "Build me a swing strategy for SOL-USD" | 30 min–hours | full research protocol, Set J | A vaulted (or honestly failed) Experiment with a strategy, surface, gate ledger, diagnostics |
| J4 | **Improve**: diagnose and evolve an existing algorithm | "Why does my EMA strategy lose in summer?" | 15 min–hours | diagnostics, regime slices, moves, sweeps | Diagnosis, then a versioned candidate compared through sealed distributions |
| J5 | **Model**: train, evaluate and wire a model into an algorithm | "Train a volatility model and use it to size my strategy" | 30 min–hours | dataset/train/evaluate/predict tools, meta-labeling | A registered model with scorecard vs baselines, and a strategy using it, evaluated in Set J |
| J6 | **Campaign**: autonomous long-running search | "Spend 6 hours finding robust strategies for BTC 1h" | hours–overnight | FEAT-003 campaign loop | Archive of candidates across strategy families, ledger of rejected hypotheses |
| J7 | **Explain**: account for a result or a decision | "Why did this backtest take that trade on the 14th?" | seconds–minutes | `explain_trade`, diagnostics | Entry-time features, regime, path, what the rule saw |
| J8 | **Monitor** (later): watch paper results against backtest expectations | "Is my paper strategy behaving like its backtest?" | continuous | paper stats, reconciliation | Drift alerts with evidence |

J1 and J2 are what make the agent feel like a quant colleague, not just a strategy
generator. They are also cheap, need no Set J changes, and are where most day-to-day use
will be. **Build them first** (see 07).

## 3. ~~Modes~~ (superseded 2026-09-10)

> **Superseded by 08 §2.1:** there is one agent, optimised for long-running work. Every
> conversation belongs to a research project with a persistent workspace; a question is
> just a short task. The table below is kept only for its budget and termination ideas.

### Former proposal: one agent, one harness, three contracts

Rather than separate agents, use one agent with three session **modes**. A mode sets the
budget, the tool groups loaded, the termination contract and the UI layout.

| | Ask | Research | Campaign |
|---|---|---|---|
| Jobs | J1, J2, J7 | J3, J4, J5 | J6 |
| Budget default | 20 tool calls, 60k tokens, 5 min | 150 tool calls, 1.5M tokens, 3 h, N backtests | backtests + wall clock + tokens (FEAT-003 §8.5) |
| Tool groups | knowledge, data, analysis | + authoring, evaluation, models | + campaign, moves, sub-agents |
| Ends with | Answer card | `final_report` (structured) | Campaign report + archive |
| User interaction | Conversational | Plan approval → checkpoints → report | Launch → notifications → report |
| Can escalate? | "This needs a research session. Start one?" | "This needs a campaign." | — |

Escalation between modes is explicit and user-approved, so a quick question never
silently turns into a six-hour compute bill.

## 4. The behaviour contract

These rules define how the agent acts. The harness enforces the ones marked **(H)**. The
prompt teaches the rest.

1. **Numbers come from tools, never from the model.** The agent may not state a statistic
   it didn't get from a tool result in this session or from research memory with a
   citation. **(H):** `final_report` claims must reference artifact/experiment ids; the
   report validator rejects uncited numbers.
2. **Performance claims come only from Set J.** Exploratory tools (`event_study`,
   `describe_returns`, sandbox) produce *evidence for hypotheses*, labelled as exploration.
   Only Experiments produce *results*. **(H)** via tool output labels.
3. **The LLM proposes structure; optimisers choose numbers** (ADR-0023). **(H)**: sweeps
   are the only way to set tunable parameters.
4. **Pre-register before testing.** Each hypothesis gets an archetype, a rationale ("who is
   on the other side, and why does this persist?"), a null, and a kill criterion before its
   first sweep. **(H)**: `record_hypothesis` is required before `create_experiment`.
5. **Look before designing.** Run the instrument profile (stylized facts, regime, costs,
   coverage) before proposing a structure. Proposals should cite the profile ("H ≈ 0.42 on
   1h returns → mean-reversion families first").
6. **Simple before complex.** Every model or complex structure names the simple baseline it
   must beat, and the comparison is run.
7. **A failed hypothesis is a result.** Rejections are recorded with their evidence so no
   future session repeats them unknowingly.
8. **Say what you don't know.** Thin history, unverified fill timing, unavailable data and
   LLM look-ahead contamination are stated in the answer, not hidden.
9. **Research authority only.** No orders, no automations, no live arming. Deployment to
   paper is a *proposal* a human approves in the UI (see 03 §J).
10. **Be economical.** Prefer one server-side analysis call over pulling data into
    context. Prefer summaries; drill down only when a decision depends on the detail.

## 5. How it interacts with the user

- **Plan first (Research/Campaign).** The agent posts a short plan: instrument profile
  highlights, 2–4 hypotheses with archetypes, budget split. The user approves, edits or
  lets it auto-approve after N seconds (configurable).
- **Checkpoints, not silence.** At meaningful events (a hypothesis killed, a candidate
  passing Gate 2, budget at 50%) the agent posts a one-line checkpoint. If configured, it
  can pause for input at a checkpoint.
- **Steering mid-run.** The user can send a message into a running session ("skip mean
  reversion, focus on breakouts"). The harness injects it at the next turn boundary. This
  doesn't exist today.
- **`ask_user` tool.** When a decision is genuinely the user's (which instrument, which
  risk tolerance, whether to spend another hour), the agent asks and waits, with a default
  and a timeout so unattended runs continue.
- **Answer cards.** Every answer has a one-paragraph conclusion, the numbers with n and
  uncertainty, caveats, and links to the artifacts (charts, experiments, strategies) the
  UI renders.

## 6. UI: the Research Workspace

Evolve `/agent` (`frontend/src/pages/AgentPage.tsx`) into a three-pane workspace:

```
┌──────────────┬─────────────────────────────────────┬───────────────────────────┐
│ Sessions     │ Conversation + timeline             │ Notebook | Artifacts      │
│  ▸ Ask       │  user / agent messages              │  Plan (checklist)         │
│  ▸ Research  │  tool cards (collapsed by default)  │  Hypotheses (status)      │
│  ▸ Campaigns │  rich cards: charts, surfaces,      │  Findings (cited)         │
│              │   gate ledgers, event studies       │  Candidates by gate       │
│ Budget meter │  checkpoints / approvals inline     │  Artifacts: charts,       │
│ (tokens,     │                                     │   datasets, models,       │
│  backtests,  │  [ steer / ask … ]                  │   strategies, reports     │
│  wall clock) │                                     │                           │
└──────────────┴─────────────────────────────────────┴───────────────────────────┘
```

- The **Notebook pane** is a live view of the harness's `ResearchState` (see 04 §3). It is
  the same object the agent reads each turn, so the user sees exactly what the agent
  "knows".
- **Dual-channel results.** Tools return a compact `for_model` text and a rich `for_user`
  payload (chart spec, table). The UI renders the rich one; the LLM only sees the caption.
  This is the single biggest token saver for Q&A (see 04 §4).
- Stream events over SSE (replacing adaptive polling) so progress, checkpoints and study
  completions appear as they happen.

## 7. What "best possible" means, measurably

Without a definition this goal can't be tested. Proposed scorecard, measured by an
evaluation suite (04 §9) on every harness or tool change:

| Dimension | Metric |
|---|---|
| Honesty | False discovery rate on pure-noise instruments (target: ≈ the gate's nominal level); rate of uncited numbers in reports (target 0) |
| Power | Detection rate on synthetic instruments with a planted edge of known size |
| Answer quality | Exact-match or tolerance accuracy on Q&A tasks with computed ground truth |
| Leakage vigilance | Share of planted leakage traps (look-ahead feature, overlapping labels) the agent flags |
| Efficiency | Tokens, tool calls and backtests per task; backtests per accepted improvement (FEAT-003 P6) |
| Robustness | Task success under tool errors, timeouts and restarts |
| User value | Time to first useful answer; share of sessions ending in a report the user keeps |
