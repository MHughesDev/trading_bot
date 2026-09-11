# BS-007 / 02 — The agent: what it is, what it does, how it behaves

**Spec status:** spec-ready draft · **Req prefix:** B- · **Formal spec:** AGENT-001 §1 ·
**Plan set:** L · **Decisions:** D-01, D-04, D-05, D-10, D-12, D-13

---

## 1. Definition

The agent answers any question about an instrument from the platform's data. It also runs
long, honest research sessions that design, test, model and explain trading strategies
for stocks and crypto. Every number it reports comes from a platform tool or a platform
artifact, and every performance claim comes from Set J.

**What it is**
- **One agent** with one tool surface, one harness and one UI (D-04).
- **Optimised for long-running work.** A question is just a short session in a project,
  and asking is a subset of what it does.
- **A coding agent in its own container** (D-02, 03). It works in files and code, and
  reaches the platform through the `tbot` SDK/CLI and MCP (14).
- **The platform's primary user** (D-05). Humans watch, steer, approve and inspect.

**What it is not**
- It is not a trader. It has no order, automation, live-arming or alias-promotion
  capability (D-12).
- It does not choose parameters. Optimisers do that (P1).
- Its own judgement is not the judge of results. Set J is (INV-1/2/3).
- It has no modes and no separate "ask" agent.

## 2. Jobs to be done

| # | Job | Example | Time scale | Ends with |
|---|---|---|---|---|
| J1 | **Ask**: answer a question about an instrument or market | "How volatile has ETH been this month vs last year, and is it trending?" | seconds–2 min | Answer card: conclusion, numbers with n/SE, caveats, chart for the user |
| J2 | **Investigate**: test an idea quickly | "Does BTC mean-revert after a 3σ hourly drop?" | 2–15 min | Conditional distribution vs a null, labelled exploration, with the exploration-ledger entry |
| J3 | **Build**: create a strategy for an instrument | "Build me a swing strategy for SOL-USD" | 30 min–hours | A vaulted or honestly failed Experiment, with dossier (11 §5) |
| J4 | **Improve**: diagnose and evolve an existing strategy | "Why does my EMA strategy lose in summer?" | 15 min–hours | Diagnosis, then a versioned candidate compared through sealed distributions |
| J5 | **Model**: train, evaluate and use a model in a strategy | "Train a volatility model and use it to size my strategy" | 30 min–hours | A registered model with a scorecard vs baselines, plus a strategy using its prediction series, evaluated in Set J |
| J6 | **Campaign**: an autonomous search | "Spend 6 hours finding robust strategies for BTC 1h" | hours–overnight | Candidate archive, rejected-hypothesis ledger, comparison against a random-search shadow arm |
| J7 | **Explain**: account for a result | "Why did this backtest take that trade on the 14th?" | seconds–minutes | Entry-time features, regime, path, what the rule saw |
| J8 | **Monitor** (after G-11): watch paper results against backtest expectations | "Is my paper strategy behaving like its backtest?" | continuous | Drift alerts with evidence |

## 3. How a session runs

1. **Project:** every conversation belongs to a research project (00 glossary). A new
   project gets an initializer session: it expands the goal into `RESEARCH_PLAN.json`,
   writes `bootstrap.sh`, and makes a baseline commit (03 §6).
2. **Start:**
   - the orchestrator materialises the skills (13 §5);
   - the agent runs `bootstrap.sh`: platform reachability, data coverage and `data_qc`,
     pending jobs, the cutoff and budget;
   - it reads the NOTEBOOK "Current state".
3. **Plan:** for multi-step work, the agent posts a short plan. The user approves, edits,
   or lets it auto-approve (§5).
4. **Work:** the agent profiles, hypothesises, pre-registers, tests through jobs,
   diagnoses and iterates. It records findings as it goes, and posts checkpoints at
   meaningful events.
5. **Finish:** the agent submits a `final_report`, which the platform validates
   (03 §10). Findings go to research memory. The Stop hook checks for the report and may
   nudge once for a skill proposal (13 §3).
6. **Resume:** state lives on disk, in the job service and in memory, so any session can
   pick up after a restart (03 §11).

A two-minute question uses the same path with a trivial plan and an answer-card report.
Questions about the present go to the user's **Desk** project (cutoff = now,
exploratory only), so they never touch a research project's holdout (06 §3.1).

## 4. Behaviour contract

| ID | Rule | Enforcement |
|---|---|---|
| B-01 | **Numbers come from tools, never the model.** Every statistic in a report cites a job, artifact, experiment or memory id | (E) `final_report` validator rejects uncited numbers and numbers that don't match the cited artifact |
| B-02 | **Performance claims come only from Set J.** Exploratory analyses produce evidence for hypotheses, labelled `exploration` | (E) epistemic class on every tool output (14 §1); validator checks class |
| B-03 | **The LLM proposes structure; optimisers choose numbers** (P1) | (E) sweeps are the only way to set tunable params; lint forbids client-side submit-and-argmax loops (13 §7) |
| B-04 | **Pre-register before testing:** mechanism, counterparty, why it persists, falsifier, null, kill criterion, alternatives considered | (E) Experiment creation refuses without a registered hypothesis (11 §2) |
| B-05 | **Look before designing:** instrument profile and `data_qc` before proposing structure | (E) Experiment creation refuses without a current `data_qc` grade ≥ C (06 §5); (A) proposals cite the profile |
| B-06 | **Simple before complex:** every model or complex structure names the baseline it must beat, and runs the comparison | (A) dossier item; (E) model scoring always includes baselines (10 §6) |
| B-07 | **A failed hypothesis is a result:** rejections are recorded with evidence | (H) Stop hook requires a finding per tested hypothesis |
| B-08 | **Say what you don't know:** thin history, data grade, fill assumptions, contamination, survivorship | (A) `final_report.caveats`; dossier items 2 and 12 |
| B-09 | **Research authority only** | (E) token scopes (03 §12); no order, automation or alias tools exist in any agent profile |
| B-10 | **Be economical without being less capable:** summaries over dumps, handles over data, code over tool chains; never skip verification to save tokens | (H) output guard; (E) tool output budgets (04) |
| B-11 | **Treat external text as data:** news, social posts, web pages and filings are read only through the quarantined reader | (E) text endpoints are callable only by the reader sub-agent's scope (03 §9) |
| B-12 | **Learn procedures, not answers:** turn verified, repeated or hard-won procedures into skill proposals; never put findings in skills | (E) skill admission (13) |

## 5. How it interacts with the user

- **Plan approval:** for multi-step work, the plan (profile highlights, 2–4 hypotheses
  with archetypes, budget split) is posted as an approval card. It auto-approves after a
  per-project timeout (default 120 s; 0 = always auto).
- **Checkpoints:** one-line events when a hypothesis is killed, a candidate passes a gate,
  budget reaches 50/80%, or a job fails repeatedly. Optionally pause at a checkpoint.
- **Steering:** the user can message a running session. The message is injected at the
  next turn boundary (03 §8).
- **`ask_user`:** for decisions that belong to the user (instrument, risk tolerance,
  spending more budget). It has options, a default and a timeout, so unattended runs
  continue.
- **Answer cards:** a one-paragraph conclusion, the numbers with n and uncertainty,
  caveats, and artifact links rendered by the UI.
- **Approvals inbox:** these are the only way consequential changes happen:
  - paper-deployment proposals (after G-11);
  - model alias promotion;
  - global skill promotion;
  - `data_qc` grade-C waivers;
  - budget increases.
- **Notifications:** approvals, completions and failures go to the UI, and optionally
  push or email.

## 6. Non-goals

- Placing or arming trades; creating automations; promoting model aliases without approval.
- Mode-specific agents or UIs.
- Provider-neutral harness (D-07). Provider neutrality remains for `LlmInference` nodes.
- Paid data sources (D-09).
- Replacing Set J statistics. The agent consumes them.

## 7. Success scorecard

Measured by the agent eval suite (16) on every harness, tool, skill or model change.

| Dimension | Metric | Target |
|---|---|---|
| Honesty | False discovery rate on pure-noise instruments (pass^k) | ≤ gate nominal level + 2 pp |
| Honesty | Uncited-number rate in reports | 0 |
| Power | Detection rate on planted-edge instruments by edge strength | Power curve tracked; no regression beyond −2 pp |
| Answer quality | Q&A accuracy against computed ground truth | Numeric tolerance per task |
| Leakage vigilance | Planted traps caught (look-ahead, overlap, collider, survivorship, post-hoc, complexity-as-vol-timing, revised data, model trained into the test window) | 100% caught by agent or Gate 0 |
| Search quality | Agent vs matched-budget random search on the sealed metric | Reported every campaign; the agent must beat it to claim value |
| Efficiency | **Cost per correct verdict** (dollars, trials, wall time); cache hit rate | Tracked; optimisations must be non-inferior on all quality rows (04 §10) |
| Robustness | Completion or resume under injected tool failures, timeouts and restarts | Session resumes with no duplicate trials |
| User value | Time to first useful answer; share of sessions ending in a report the user keeps | Tracked |

## 8. Requirements (summary)

| ID | Requirement | Pri |
|---|---|---|
| B-R1 | Every conversation MUST belong to a research project with its own workspace, cutoff, budget and memory scope | MUST |
| B-R2 | The agent MUST expose the same capabilities regardless of task length; nothing is gated by "mode" | MUST |
| B-R3 | Rules B-01…B-12 MUST be enforced at the point listed in §4 | MUST |
| B-R4 | Every session MUST end with a validated `final_report` or an explicit abort record | MUST |
| B-R5 | Steering, `ask_user`, plan approval, checkpoints and the approvals inbox MUST exist in v1 | MUST |
| B-R6 | The §7 scorecard MUST be computed by the eval suite and shown per harness version | MUST |
