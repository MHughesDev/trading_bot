# Knowledge and autonomy — Set R-b

**Completion: 47% left (14 / 34 tasks). 14 done, 14 partial, 6 BLOCKED:UPSTREAM, 0 not started.**

**Status:** PLANNED (2026-10-04, track F-RESERVED) — this file was the reserved letter in [ROADMAP §5](../../ROADMAP.md#5-set-succession-what-the-reserved-letters-still-mean) until today.
**Stage:** [5](../../ROADMAP.md#4-the-stages) — accumulated knowledge
**Derived from:** [AGENT-003](../../../specs/AGENT-003-knowledge-memory-skills.md) §3–§7, [BS-007 12_KNOWLEDGE_MEMORY](../../../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/12_KNOWLEDGE_MEMORY.MD), [FEAT-003](../../../specs/FEAT-003-strategy-research-agent.md) §10 and §15, ML pack PHASE-2-5-PLAN §4 (ADR-P4-01), ROADMAP §5, OPEN-TASKS Part C3
**Requirement IDs:** KM-02…KM-10, SK-11, SK-16, SK-17, EV-09, FEAT-003 phases 2 to 4.
**Migration range:** 0213–0214.
**Scope class:** End-state architecture.

---

## 1. Why this set exists

This is the set that makes the platform's experience accumulate: memory that retrieves by what matters and shows how stale it is, skills that are promoted on evidence and retired when they stop earning, a random-search arm
that keeps a campaign honest about what it found, a regime engine that says what market the strategy is in, and the learned research-process models that become worth fitting only when the ledger is large enough to train them.
Most of it is deliberately **behind a trigger or a prerequisite**. Set S built the harness and the stores; Set R-a builds the registry; the ledger has not yet grown to the sizes the learned tiers need.

## 2. Scope

| Area | Today | This set delivers |
|---|---|---|
| Knowledge | findings, insights, a local embedder (Set S) | move memory with a veto, the curator, analog search, tainting, a hybrid store decision |
| Skills | none beyond core | promotion to global on six conditions, quarantine, A/B, re-evaluation, retirement |
| Campaign honesty | a campaign reports its own best | a matched-budget random arm reported beside it |
| Regimes | regime jobs and a separation test | a filtered jump-model engine, a prior table, tools and a gate move |
| Learned tiers | rule tiers only | M3, M4, M5, M8, M9, M13 behind computed triggers |
| FEAT-003 | Phase 1 shipped; outer loop partly landed | islands, bandit, multi-instrument, refit, drift alerts, racing |

### Out of scope

- The registry, materialiser and propose/verify path: Set R-a.
- The autonomy and continuous-research harness: Set S Phases 10 and 11.
- KM-01 (findings write path): Set S S-4.2.
- Live capital, deployment: Set T.

### Excluded because something else already owns it

| What | Owner | Evidence |
|---|---|---|
| KM-01 findings through `tbot memory add` | Set S | S-4.2 |
| Campaigns as the `optimise` workflow; autonomy as continuous research | Set S | Phases 10 and 11 |
| The `InstrumentProfile` writer and the insight store endpoints | track CAMPAIGN | 2026-10-03 entries in the ML checklist |
| Walk-forward regime *family* training | Set N | N-2.4 |

## 3. Decisions made while planning (ADR rows)

Each row records the alternative that was rejected. They are this set's own ADR index; promote one to `docs/adr/` when a task relies on it.

| ID | Decision | Rejected alternative | Why |
|---|---|---|---|
| ADR-RB-01 | Prefer pgvector plus Postgres full-text over standing up Milvus, subject to the owner's approval of the extension (RB-1.6). | Deploy Milvus as AGENT-003 names. | One fewer service to run on a single-tenant platform; hybrid retrieval over a few thousand findings does not need a distributed vector store. |
| ADR-RB-02 | Learned tiers are built only when their ledger-size trigger is computed true. | Build them now against synthetic data. | ADR-P4-01: a learned model on a ledger too small to train it is confidently wrong. |
| ADR-RB-03 | The random arm is part of the campaign record, not a separate study. | A free-standing `compare_to_random` run after the fact. | A comparison run after the fact can be re-rolled until it flatters; a matched arm created with the campaign cannot. |
| ADR-RB-04 | KM-02, KM-03, KM-05, KM-06 are verify-and-close tasks, not builds. | Rebuild them from the spec. | Set S built most of them under other names; the defect this repo breeds is code with no caller. |
| ADR-RB-05 | Regime labels are filtered only; a smoothed label has no code path. | Offer smoothed labels for analysis. | A smoothed label uses the future; it is a look-ahead on a table the strategies read. |
| ADR-RB-06 | **KM-07 / RB-1.6 decided: stay on pgvector plus Postgres full-text. Milvus remains deferred.** | Deploy Milvus as AGENT-003 names. | Migrations 0055, 0122 and 0125 already require the `vector` extension, so the platform's database is a pgvector database; a few thousand findings and skills do not need a second, distributed store; and the registry's lexical search already runs on Postgres full-text. Revisit on a measured recall or latency failure, with the dense leg added to `skills::registry::similarity` and `routes/skills::search`. |
| ADR-RB-07 | Analog search excludes, on the **same instrument**, any window within one window length of the query's, and the query's own episode by id; windows on other instruments at the same time are legitimate analogs. | Excluding every overlapping window across all instruments. | The nearest neighbour of a window is its own overlap; a different market at the same moment is a second observation, not the same episode. |
| ADR-RB-08 | The random arm draws the **declared** search space, ignoring the agent's narrowing. | Giving the arm the agent's narrowed space. | A narrowing is the agent's judgement about where to look; a baseline that inherited it would hide the agent's best idea inside what it is compared with. |
| ADR-RB-09 | Baum-Welch's backward pass runs inside `regime_learned::fit` to estimate parameters on the training window; no function returns its output as a label. The jump model's fit updates centroids from *filtered* assignments, so it has no backward pass at all. | Forbidding the backward pass outright (no HMM EM), or exposing smoothed labels for analysis. | EM needs it; the invariant is about what a strategy can read (ADR-RB-05), and two source-scan tests assert that no public item smooths. |
| ADR-RB-10 | Taint (KM-10) is the `extra.tainted` key of an `agent_memory.entry`, read by retrieval SQL as `(extra ->> 'tainted')::boolean`. | A `tainted` column. | No `Entry` literal in the workspace changed, a database that has not applied a migration still reads and writes every entry, and `skill_provenance` computes the mark from the store at citation time. |
| ADR-RB-11 | Platform measurements (a move's outcome, a regime-sliced study's score, a trial's sealed score, a random arm) are written under `knowledge.write`, which a web login holds and no agent token can. | Letting the agent record its own moves and scores. | An agent that records its own outcomes can dilute the failures that veto its favourite move, or hand `compare_to_random` a flattering list. |
| ADR-RB-12 | The model-change guard (SK-17) runs where a session starts: a model with no completed library re-evaluation gets core skills only, and skills failing under it are withheld and listed. | Refusing the default model change in `agent/routes.rs` (not this set's file) or at deploy. | The default model is an environment variable with no change event to hook; withholding at session start enforces the same property and is visible in the charter. |
| ADR-RB-13 | M7's trigger is "the rule tier's out-of-sample separation test has passed" (PHASE-2-5-PLAN row M7). | A ledger-size count for the regime tier. | The plan states M7 as "HMM when 3.5's validation passes"; a count of studies would trigger a model of labels that may mean nothing. |
| ADR-RB-14 | The jump model writes labels to `regime_causal.regime_state` under `model_version = jump_v1:<fit date>`, fits to `regime_causal.regime_jump_model` (0214). | A new `regime_labels` table as the spec's §9 sketch names. | `regime_state` already carries filtered labels, `model_version` and a causal `knowledge_time`, and the rule tier and the jump model must be comparable in one table. |
| ADR-RB-15 | A new platform write scope, `knowledge.write`, is not added to `auth/scopes.rs`; it is held only by `web:full`, and `validate_agent_scopes` already refuses any scope outside `RESEARCH_SCOPES`. | Editing `scopes.rs` (outside this set's territory). | The invariant test `skills_admit_is_never_an_agent_scope` asserts it from the outside. |
| ADR-RW-01 | The move veto is consulted in `CampaignWorker::advance` before an EXPERIMENT child is created; an un-overridden block returns the terminal `move_vetoed` (citing the records) and appends nothing, so the fold resumes at the same position after a person records an override. A warn travels in the EXPERIMENT event's detail. | Vetoing in `log_proposal` after HYPOTHESIZE, or appending the EXPERIMENT event and skipping its child. | The HYPOTHESIZE event is already in the log when `log_proposal` runs, so a resume would skip it and never re-ask; an EXPERIMENT event with no child makes COMPARE fail `no_experiment_to_compare`. Failing before the append leaves nothing to unwind. |
| ADR-RW-02 | A COMPARE result is the move's outcome: promoted is `improved`, a `reject` verdict is `failed`, and everything else (equivalent, undecided, no feasible candidate) is `neutral`. | Counting "no feasible candidate" or "equivalent" as a failure. | A sweep that found nothing feasible says nothing about the structural move, and counting it would let search difficulty build a veto against a sound idea. Neutral counts toward neither side, so it can neither build a veto nor dilute one. |
| ADR-RW-03 | The matched arms cover the campaign's **first** EXPERIMENT: arm budget = the declared sweep size (`max_runs`), and later generations record no arm scores. | Growing the arm every generation. | `campaign_arm.budget` is fixed at creation and `compare_to_random` refuses unequal budgets, so a moving target would make the arm "complete" only by accident and would let Gate 3 pass or fail on when it was read. The cost is stated in RB-3.1: a many-generation campaign is compared on its first sweep only. |
| ADR-RW-04 | A sweep that drew fewer points than the budget is padded to the budget with `-inf` (the failure value) for both arms by one function, `matched_scores`. | Recording the shorter list, or shrinking the arm's budget. | A shorter agent list is refused as `UnequalBudget` and the arm would never complete; an undrawn trial is a trial that scored nothing, and `compare_to_random` already ignores non-finite scores. |
| ADR-RW-05 | Per-sample scores are carried in memory (`SweepReport::sample_scores`, `SweepOutcome::sample_scores`, both `#[serde(skip)]`) and the platform writes them to the arm table. | Adding the scores to the serialised sweep report. | INV-2: the report an agent reads holds no ranked list and no argmax, and a score per sample is both. A test asserts the serialised report has no `sample_scores`. |
| ADR-RW-06 | Gate 3's arm requirement is **data**: `api::gate_worker` asks `knowledge_routes::arm_gate` and hands `backtest::gates::random_arm_gate` an `ArmRequirement`, which `apply_to_stack` applies (the same seam as the reconciliation gate). | A trait in `backtest` that `api` implements and the stack awaits; or `backtest` calling `research`. | `research` depends on `backtest`, so the second is a cycle; the first would make a pure, synchronous stack evaluation await a database. The rule stays in one place (`research::random_arm::gate_check`). An unreadable arm is a refusal. |
| ADR-RW-07 | A regime-sliced study is the COMPARE phase's replicate windows of the candidate: each window is labelled with the filtered regime in force at its **start**, success is a positive primary metric, and the fold runs after the decision is logged (advisory: a failure is logged). | Folding in the EXPERIMENT sweep, or labelling a window by its end or its majority regime. | The sweep's runs are not regime-sliced and expose no per-window scores; the end or the majority of a window is known only after it, which is a look-ahead for a prior that conditions the next decision. Folding after the decision keeps a retry from counting the same windows twice. |
| ADR-RW-08 | The rule tier's volatility terciles are expanding (cut points from days up to and including `t`) and the trailing window is not clipped to the series length. | A fixed burn-in that returns no label for early days. | Callers rely on one label per day; the early labels are coarse, which is what a filter knows on its first days. A burn-in would change the length of every consumer's input. |
| ADR-RW-09 | A calibration drift's inbox id is a hash of the model and the end of the window that completed the pair; items are derived from `model_quality_series` on read, not stored. | A stored alert table written by the quality monitor. | A second copy of what the series already says can disagree with it; the id is stable so acknowledge and dismiss (stored in `alert_inbox_state`) refer to the same item, and a longer run of bad windows does not mint a second alert. |
| ADR-RW-10 | `evidence_ids` on a hypothesis or an experiment request exists to be refused when it names a finding; any other ids are accepted and not stored. | Persisting the list. | The platform derives a hypothesis's evidence from tickets, so a stored list would be a second, unverified account of it. |
| ADR-RW-11 | The curator run stays person-triggered (`POST /api/skills/curator/run`, `tbot`); there is no timer. | A daily job in the platform. | Its items wait for a person, so a timer only grows an unreviewed queue; every acceptance clause is met without it. |
| ADR-RW-12 | `GET /api/skills/glossary` and `/search` return each skill's `skill_id` (additive), read under the glossary's own visibility predicate. | Addressing skills by name on the quarantine and item routes. | Those routes are by id so a rename cannot retarget a quarantine; the page needs the id and the materialiser's rows carry none. |

## 4. Phase summary

| Phase | Title | Tasks | Migrations | State |
|---|---|---|---|---|
| 1 | [Knowledge and memory (KM-02 to KM-10)](phase-1-knowledge.md) | 9 | 0213 | 5 / 9 done |
| 2 | [Skill promotion, curation and evaluation](phase-2-skill-promotion-and-evaluation.md) | 6 | none | 4 / 6 done |
| 3 | [The shadow random-search arm (EV-09)](phase-3-random-arm.md) | 3 | none | 2 / 3 done |
| 4 | [The regime engine and its learned tier](phase-4-regime-engine.md) | 6 | 0214 | 1 / 6 done |
| 5 | [Learned model tiers, FEAT-003 phases 2 to 4 and the visual pipeline builder](phase-5-learned-tiers-and-campaigns.md) | 10 | none | 2 / 10 done |

### Reading the statuses

> **Verified 2026-10-05:** skills lib 114, research lib 150, harness lib 708, api lib targeted 40, mcp-server-lib catalogue tests (incl. the `propose_skill` descriptor budget), static_invariants 35, `skills_admit_is_never_an_agent_scope` 6, pipeline-builder guard 2, `skills_registry_flow` 13 and `pg_registry` 9 on Postgres 5433, `quality_live` 3. Anything needing pgvector remains compile-checked only.

Nothing in this set was built by F-RESERVED. **Track R-SKILLS built what is buildable on 2026-10-04**, against the registry it built for Set R-a:

- *Skills* (Phase 2): promotion to global over six recorded conditions with a taint check computed from the memory store, project-to-user promotion with an audit log, quarantine with a same-statement notice, the sequential
  paired-evaluation design and the retirement sweep (both wired into session start), the model-change guard, alias resolution of deprecated skills.
- *Knowledge* (Phase 1): the retrieval fix for KM-02 (it embedded before it filtered, and returned no staleness or capability flags), the local-embedding guard and per-vector model and dimension (KM-06), taint (KM-10),
  hypothesis sources and repeat flagging (KM-03), move memory with a confidence-gated veto (KM-04), analog search over the stored fingerprints (KM-08), the curator (KM-09), and the **RB-1.6 decision: pgvector plus
  Postgres full-text, Milvus deferred (ADR-0049)**.
- *The random arm* (Phase 3): the matched-budget arm, its store and the comparison report.
- *Regimes* (Phase 4): the audit (RB-4.1, which found the rule tier's terciles are not filtered), a filtered statistical jump model and its persistence, the HMM and change-point detector, the prior table, the profile and
  prior tools and the regime-gate check.
- *Learned tiers and campaigns* (Phase 5): every trigger is now computed from the ledger (`GET /api/knowledge/tiers`) and the learned models stay `BLOCKED:UPSTREAM` on the data; the campaign archive, islands and bandit,
  the cost ladder, drift detector and racing report exist as pure rules; RB-5.10 is closed as "not yet" with a test that re-opens it.

**R-WIRE, 2026-10-05, wired the hand-offs this paragraph used to list:** the campaign driver (move veto and records, the arm created with the campaign), the study and compare workers (both arms, the regime prior), Gate 3 (the arm requirement on the sixteen-gate stack), the daily regime job (the jump labeller), the filtered rule-tier terciles, the alert inbox (a `calibration` source, migration 0245), `sweep_slots` and the evidence refusal on the hypothesis and experiment routes. ADR-RW-01 to ADR-RW-12 are its decisions. What is still not true is stated per task: nothing here has run against the real Backtest Suite and ClickHouse (every driver and worker test uses a fake engine), the arms are matched on the first generation only, multi-instrument campaigns and the islands/bandit allocation are not in the driver, and the RB-4.4 citation of the prior by a first-generation hypothesis is in `agent/**`.

What a reader should not infer from the counts (written before R-WIRE; what it lists as outside this track is now wired except where a task says otherwise): the **callers in files outside this track** (the campaign driver, Gate 3, the study worker, `regime_jobs.rs`, the experiment and hypothesis routes, the alert inbox) are not
edited, so each task that needs one is `PARTIAL` and its exact line is in the track report. Anything that needs **pgvector** (the insight and regime SQL) is compile-checked and not executed on this machine.
