# BS-007 / 04 — The harness

Status: draft proposal. Builds on `crates/api/src/agent/{driver,manager,prompt,routes}.rs`
(see 01 §8 for what exists).

> **Partly superseded by 08 §2:** the recommendation is now to *replace* the Rust driver
> with a coding-agent runtime in a per-project container, not extend it. Still valid here:
> token-efficiency levers (§4), sub-agent roles (§6), phase gates (§7, now enforced by the
> platform and hooks), the agent evaluation suite (§9), durability (§10), and the
> `final_report` contract (§11). `ResearchState` becomes a workspace `NOTEBOOK.md`.
>
> **Skills (12):** at session start the orchestrator materialises core, global, user and
> project skills plus a registry-generated `GLOSSARY.md`. A Stop hook nudges skill
> proposals on the T1–T3 triggers. A PreToolUse hook denies writes to `.claude/skills/**`.
> The isolated Reviewer is added to the §6 sub-agent roles for `skill_verify`.

---

## 1. Shape

```
                ┌───────────────────── Session (Ask | Research | Campaign) ─────────────────────┐
 user ⇄ UI ⇄ SSE │  Orchestrator loop (strong model)                                           │
                │   context = [cached system core] + [tool groups] + [ResearchState] +         │
                │             [rolling summary] + [recent turns]                               │
                │     │ tool calls                         ▲ compact for_model results         │
                │     ▼                                    │                                   │
                │  Tool router ──► platform /api/* (via dispatch_tool)   Job manager ◄── NATS  │
                │     │                                                   (durable jobs,       │
                │     ├──► sub-agents (Analyst / Author / Critic / Summarizer)  zero-token wait)│
                │     └──► artifact store (handles)                                            │
                │  Checkpointer: ResearchState + pending jobs → Postgres (resume on restart)    │
                └──────────────────────────────────────────────────────────────────────────────┘
```

## 2. The loop

Keep the existing driver (loopback tools, run-scoped service token, budgets, persisted
transcript, zero-token waits). Change what goes into the model's context and how a run
survives time.

Turn structure:
1. Apply queued user steering messages.
2. Assemble context (§3, §4).
3. Call the LLM. Tool calls fan out in parallel where independent (today they're serial).
4. Slow tools become jobs. If the model's only pending actions are waits, the harness
   sleeps on job-completion events (NATS) at zero token cost, then wakes the model with a
   compact completion notice.
5. Update `ResearchState` from `record_*`/`update_plan` calls, checkpoint, stream events to
   the UI.
6. Check budgets and the phase gate (§7).

## 3. Working memory: `ResearchState` instead of "the transcript is the memory"

Today the whole transcript is resent every turn, and old tool results are elided only
past 400k chars. The model's memory is whatever survives elision. Replace it with an
explicit, harness-owned document injected at the top of each turn:

```jsonc
ResearchState {
  goal, mode, instrument_profile_ref,
  plan: [{ step, status }],
  hypotheses: [{ id, archetype, statement, null, kill_criterion, status, experiment_ids }],
  findings:   [{ id, claim, evidence: [artifact/experiment ids], class, confidence }],
  candidates: [{ strategy, experiment_id, gate_reached, worst5, median, trials }],
  open_questions: [...],
  budget: { tokens, tool_calls, backtests, wall_clock, spent }
}
```

- The agent writes to it only through tools (`update_plan`, `record_hypothesis`,
  `record_finding`), so it stays structured and citeable.
- Rendered compactly (~1–3 KB) each turn. It replaces most of the history the model
  would otherwise need.
- Older turns are replaced by a **rolling summary** produced by a cheap Summarizer model
  when the recent-turn window overflows. The raw transcript stays in `agent_messages` for
  the UI and audit.
- The UI Notebook pane renders the same object (02 §6).

## 4. Token efficiency: the levers, ranked by expected savings

| # | Lever | Where | Expected effect |
|---|---|---|---|
| 1 | **Prompt caching** of system core + tool definitions (Anthropic `cache_control`, OpenAI automatic prefix caching) | `crates/llm/src/anthropic.rs` has none today | Largest cost cut on long sessions: the stable prefix is re-read every turn |
| 2 | **Dual-channel results**: charts/tables to the UI, captions to the model | tool layer | Q&A answers stop carrying data arrays |
| 3 | **Server-side analysis tools** instead of `get_bars` dumps | 03 §D | One 1.5 KB report replaces ~30 KB of bars |
| 4 | **`ResearchState` + rolling summary** instead of full transcript | §3 | Context stays roughly flat over a long session instead of growing |
| 5 | **Tool groups loaded by mode/phase** | §5 | 38 → ~15 schemas in context for Ask mode |
| 6 | **Sub-agents with summary returns** | §6 | Exploration noise stays out of the orchestrator's context |
| 7 | **Split the authoring guide into cards**, loaded on demand | `prompt.rs` embeds the whole guide today | Smaller cached core |
| 8 | **Model routing**: cheap/local model for summarising, JSON repair, simple Ask answers | §8 | Lower cost per token on bulk work |
| 9 | **Compact encodings**: TSV tables, fixed decimals, ids instead of repeated objects | tool layer | 20–40% on tabular results |

## 5. Tool loading

- **Core (always):** session/meta (03 §A), `search_knowledge`, `get_instrument_profile`.
- **By mode:** Ask loads data + analysis. Research adds authoring + evaluation (+ models on
  demand). Campaign adds moves and sub-agent tools.
- **On demand:** `load_toolset("models")` etc. The harness reports loaded groups in
  `ResearchState` so the model knows what it has.
- For local models (Ollama), cap loaded schemas at ~12–15 and prefer coarse tools. This is
  the main reason the internal profile excludes the draft builder today.

## 6. Sub-agents

Short-lived, focused contexts that return a compact result and are then discarded.

| Role | Gets | Returns | Why |
|---|---|---|---|
| **Analyst** | A question + data/analysis tools | Answer card with cited numbers | Q&A inside a research session without polluting the orchestrator |
| **Author** | A structural spec (archetype, features, move) + authoring tools | A validated strategy slug + explanation | JSON wrangling and validator retries are token-heavy and mechanical |
| **Critic (red team)** | A candidate + evaluation tools + the Experiment's ledger | A falsification report: which nulls/robustness studies to run, what failed | Adversarial pressure; counters the orchestrator's confirmation bias |
| **Summarizer** | Transcript slice or big tool output | Summary ≤ N tokens | Cheap model; powers §3 rolling summary |

Every Run a sub-agent causes goes through the same Experiment and counters (INV-1). Give
sub-agents their own token budget carved from the session's.

## 7. Phase gates: the protocol as a state machine

Today the research protocol is prompt text. Make the important orderings mechanical:

```
PROFILE → HYPOTHESIZE → PRE-REGISTER → SWEEP → DIAGNOSE → (MOVE → SWEEP)* → VALIDATE → GATE → REPORT
```

- `create_experiment` refuses without a recorded hypothesis whose null is chosen.
- `advance_gate` to G3 refuses without `choose_null` (Set J already enforces this).
- `final_report` refuses uncited numbers and requires every hypothesis to have a status.
- Phases only gate *order*; the model still decides what to do inside each phase.

## 8. Model routing

| Work | Model tier |
|---|---|
| Planning, hypotheses, diagnosing, final report | Strongest available (Claude Opus/Sonnet class or GPT equivalent) |
| Author sub-agent (JSON from a spec) | Mid tier |
| Summaries, JSON repair, simple Ask answers | Small or local (Haiku class / Ollama) |

`crates/llm` already abstracts providers. Routing is a per-role model setting in
`AgentConfig`.

## 9. Evaluating the agent itself

"Best possible agent" needs a benchmark the harness runs in CI or nightly.

| Suite | Construction | Pass criterion |
|---|---|---|
| **Planted edge** | Synthetic instruments: GARCH-t noise + a known signal (AR(1) φ = 0.05 at 1h; hour-of-day drift; vol-breakout drift) at several strengths | Detection rate vs strength (power curve) |
| **Pure noise** | Same generator, no signal | Reports "no robust edge" at ≈ the gate's nominal false-positive rate |
| **Leakage traps** | A feature that peeks one bar ahead; overlapping labels; a model trained on the test window | The agent (or Gate 0) flags it |
| **Q&A accuracy** | Questions with computed ground truth ("30-day realised vol of X", "largest drawdown in 2025") | Numeric tolerance |
| **Robustness** | Injected tool failures, timeouts, a mid-run platform restart | Session completes or resumes correctly |
| **Efficiency** | All of the above | Tokens, tool calls, backtests per task tracked over time |

Synthetic instruments need a way to register a generated series as a backtestable
instrument (a `synthetic` venue in ClickHouse). This is also useful for users.

## 10. Long-running and durable

- **Resume, don't fail, on restart.** Checkpoint `ResearchState`, pending job ids and the
  rolling summary. On boot, resume sessions instead of marking them failed
  (`recover_orphans` today).
- **Durable jobs.** Sweeps and suites live in memory today (FEAT-003 deferred). Persist
  job manifests so a restart doesn't orphan compute or double-count trials.
- **Notifications.** Push/email/UI notification on checkpoints that need input and on
  completion.
- **Idempotent submissions.** A retried `start_*` with the same manifest hash returns the
  existing job.
- **Budgets in real units.** Tokens, dollars (per provider pricing), backtests, wall
  clock. The UI shows burn rate.

## 11. Termination: `final_report`

```jsonc
final_report {
  answer: "one paragraph",
  outcome: "vaulted | failed_gate_n | inconclusive | answered",
  claims: [{ text, evidence: ["exp_…", "art_…"], class }],
  candidates: [{ strategy, experiment_id, gate_reached, trials }],
  rejected: [{ hypothesis_id, reason, evidence }],
  caveats: [...],          // data depth, fill-timing, contamination
  next_steps: [...]
}
```

The harness validates it (ids exist, numbers match the cited artifacts within
rounding), stores it, and writes the findings to research memory (05).
