# OPEN-TASKS — every written task that is not yet done

**Snapshot taken 2026-10-05 (track VERIFY2, after the 2026-10-04/05 push).** This lists every task that is
written down somewhere in the plans, specs or backlog and is not `DONE`. It is a worklist, not a progress
report. [PLAN.md](PLAN.md) still holds the percentages and the order of work, and its counts come from
`the_plan_counts_what_the_plans_say`. Don't copy numbers from here into PLAN.

Sources:
- **Part A** comes from the `### <TASK-ID>` status markers in sets P, S, T, U, M, N, Q, R-a and R-b,
  extracted by script (task ids `M-`, `N-`, `Q-`, `RA-`, `RB-` for the new sets). Regenerate it from the phase
  files; do not edit the tables by hand.
- **Part B** comes from the older sets, which have no per-task markers: C, D, E, G, H and L. Their
  MASTER and phase files were updated by the 2026-10-03/04 tracks and are the source here.
- **Part C** is work written in ROADMAP, the specs, the queue, the backlog and the ML context pack
  that has **no task in any plan file**, with what the push settled marked as done.

Part A uses the markers exactly as the invariant test reads them (a status in backticks at the start of one of the
five lines after the heading), so a task whose note mentions a human gate but whose marker says
`BLOCKED:UPSTREAM` (T-0.4) is listed under its marker.

A `PARTIAL` marker can mean anything from "complete except for one database test" to "the types
exist and nothing calls them". Before picking one up, read its note in the phase file.

---

## Part A — active plan sets (156 open)

### Set P — Data Reach — 8 open (5 PARTIAL, 2 BLOCKED:HUMAN, 1 BLOCKED:UPSTREAM)

| task | title | status | file |
|---|---|---|---|
| P-0.4 | Crypto option chain snapshots, forward — S | `BLOCKED:UPSTREAM` | [phase-0-measure-and-collect.md](plan-sets/set-P/phase-0-measure-and-collect.md) |
| P-1.3 | Kraken OHLC and downloadable history files — M | `PARTIAL` | [phase-1-deep-backfill.md](plan-sets/set-P/phase-1-deep-backfill.md) |
| P-1.5 | Revisions retained, never overwritten — M | `PARTIAL` | [phase-1-deep-backfill.md](plan-sets/set-P/phase-1-deep-backfill.md) |
| P-3.1 | FRED / ALFRED with vintages — M | `BLOCKED:HUMAN` | [phase-3-alternative-data.md](plan-sets/set-P/phase-3-alternative-data.md) |
| P-3.2 | SEC EDGAR fundamentals, PIT by acceptance time — M | `PARTIAL` | [phase-3-alternative-data.md](plan-sets/set-P/phase-3-alternative-data.md) |
| P-3.3 | Alpaca news and corporate actions — S | `BLOCKED:HUMAN` | [phase-3-alternative-data.md](plan-sets/set-P/phase-3-alternative-data.md) |
| P-3.5 | Text PIT rules, enforced — M | `PARTIAL` | [phase-3-alternative-data.md](plan-sets/set-P/phase-3-alternative-data.md) |
| P-3.6 | Macro and fundamentals as PIT feature inputs — S | `PARTIAL` | [phase-3-alternative-data.md](plan-sets/set-P/phase-3-alternative-data.md) |

### Set T — Forward Evidence — 10 open (9 PARTIAL, 1 BLOCKED:UPSTREAM)

| task | title | status | file |
|---|---|---|---|
| T-0.1 | Wire `crates/reconciliation` to the paper fill stream — M | `PARTIAL` | [phase-0-reconciliation.md](plan-sets/set-T/phase-0-reconciliation.md) |
| T-0.4 | Deploy one hand-authored strategy and start the clock — M | `BLOCKED:UPSTREAM` | [phase-0-reconciliation.md](plan-sets/set-T/phase-0-reconciliation.md) |
| T-1.1 | `Proposal` as the only door into ring 2 — M | `PARTIAL` | [phase-1-deployment-path.md](plan-sets/set-T/phase-1-deployment-path.md) |
| T-1.3 | Deployment is visible, and revocable in one action — M | `PARTIAL` | [phase-1-deployment-path.md](plan-sets/set-T/phase-1-deployment-path.md) |
| T-1.4 | `legacy_ungated` markers on everything already running — M | `PARTIAL` | [phase-1-deployment-path.md](plan-sets/set-T/phase-1-deployment-path.md) |
| T-1.5 | Retire `POST /api/backtests` — M | `PARTIAL` | [phase-1-deployment-path.md](plan-sets/set-T/phase-1-deployment-path.md) |
| T-2.3 | The monitor runs in production — M | `PARTIAL` | [phase-2-paper-fleet.md](plan-sets/set-T/phase-2-paper-fleet.md) |
| T-3.1 | The ladder, as a state machine — M | `PARTIAL` | [phase-3-ramp-mechanics.md](plan-sets/set-T/phase-3-ramp-mechanics.md) |
| T-3.2 | Gate 16 judges the step — M | `PARTIAL` | [phase-3-ramp-mechanics.md](plan-sets/set-T/phase-3-ramp-mechanics.md) |
| T-3.3 | Step-down: 50% per period, to zero — M | `PARTIAL` | [phase-3-ramp-mechanics.md](plan-sets/set-T/phase-3-ramp-mechanics.md) |

### Set U — Unified Strategy Engine — 5 open (2 PARTIAL, 2 BLOCKED:UPSTREAM, 1 NOT STARTED)

| task | title | status | file |
|---|---|---|---|
| U-5.8 | One model per walk-forward fold — L | `PARTIAL` | [phase-5-models.md](plan-sets/set-U/phase-5-models.md) |
| U-7.7 | A deployment proposal a human decides — M | `NOT STARTED` | [phase-7-authoring.md](plan-sets/set-U/phase-7-authoring.md) |
| U-8.3 | Retire the old evaluators — M | `BLOCKED:UPSTREAM` | [phase-8-parity-and-retirement.md](plan-sets/set-U/phase-8-parity-and-retirement.md) |
| U-8.4 | Retire the duplicate grammars — M | `BLOCKED:UPSTREAM` | [phase-8-parity-and-retirement.md](plan-sets/set-U/phase-8-parity-and-retirement.md) |
| U-8.5 | Migrate stored strategies — M | `PARTIAL` | [phase-8-parity-and-retirement.md](plan-sets/set-U/phase-8-parity-and-retirement.md) |

### Set S — Agent Platform — 49 open (48 PARTIAL, 1 BLOCKED:HUMAN)

| task | title | status | file |
|---|---|---|---|
| S-0.1 | Sort the working tree with the user — S | `PARTIAL` | [phase-0-ground-rules.md](plan-sets/set-S/phase-0-ground-rules.md) |
| S-0.3 | Commit the untracked catalogue, under the tracked path casing — S | `PARTIAL` | [phase-0-ground-rules.md](plan-sets/set-S/phase-0-ground-rules.md) |
| S-0.5 | Update every index that now points at nothing — S | `PARTIAL` | [phase-0-ground-rules.md](plan-sets/set-S/phase-0-ground-rules.md) |
| S-1.2 | `ToolResult`: answers, not data — M | `PARTIAL` | [phase-1-contracts.md](plan-sets/set-S/phase-1-contracts.md) |
| S-1.19 | The error contract, and not writing twice — M | `PARTIAL` | [phase-1-contracts.md](plan-sets/set-S/phase-1-contracts.md) |
| S-2.5 | `agent_events`: the effect log is the run — L | `PARTIAL` | [phase-2-one-runtime.md](plan-sets/set-S/phase-2-one-runtime.md) |
| S-2.6 | Resume from the log — M | `PARTIAL` | [phase-2-one-runtime.md](plan-sets/set-S/phase-2-one-runtime.md) |
| S-2.8 | Columns the later phases read — S | `PARTIAL` | [phase-2-one-runtime.md](plan-sets/set-S/phase-2-one-runtime.md) |
| S-2.9 | Flag the frontier onto the loop, and prove it before flipping — M | `PARTIAL` | [phase-2-one-runtime.md](plan-sets/set-S/phase-2-one-runtime.md) |
| S-2.11 | Waiting costs nothing — M | `PARTIAL` | [phase-2-one-runtime.md](plan-sets/set-S/phase-2-one-runtime.md) |
| S-2.12 | Budgets, watchdog, approvals, steering, escalation, sub-runs — L | `PARTIAL` | [phase-2-one-runtime.md](plan-sets/set-S/phase-2-one-runtime.md) |
| S-2.14 | Steering that changes the goal; one live turn per conversation — M | `PARTIAL` | [phase-2-one-runtime.md](plan-sets/set-S/phase-2-one-runtime.md) |
| S-2.15 | Draining: a deploy with live runs — M | `PARTIAL` | [phase-2-one-runtime.md](plan-sets/set-S/phase-2-one-runtime.md) |
| S-2.16 | Cost is an approval axis — S | `PARTIAL` | [phase-2-one-runtime.md](plan-sets/set-S/phase-2-one-runtime.md) |
| S-3.10 | `future_perturbation_test` — M | `PARTIAL` | [phase-3-catalogue.md](plan-sets/set-S/phase-3-catalogue.md) |
| S-3.14 | Venue constraints, funding, divergence attribution — L | `PARTIAL` | [phase-3-catalogue.md](plan-sets/set-S/phase-3-catalogue.md) |
| S-3.16 | What the signal actually is — L | `PARTIAL` | [phase-3-catalogue.md](plan-sets/set-S/phase-3-catalogue.md) |
| S-3.17 | Model diagnostics — M | `PARTIAL` | [phase-3-catalogue.md](plan-sets/set-S/phase-3-catalogue.md) |
| S-3.19 | Spending guards: call budgets, cost preview, self-status — M | `PARTIAL` | [phase-3-catalogue.md](plan-sets/set-S/phase-3-catalogue.md) |
| S-4.6 | The user's own knowledge — S | `PARTIAL` | [phase-4-memory-and-evaluation.md](plan-sets/set-S/phase-4-memory-and-evaluation.md) |
| S-4.8 | Trajectories and tapes — L | `PARTIAL` | [phase-4-memory-and-evaluation.md](plan-sets/set-S/phase-4-memory-and-evaluation.md) |
| S-4.12 | Calibrate the estimator — M | `PARTIAL` | [phase-4-memory-and-evaluation.md](plan-sets/set-S/phase-4-memory-and-evaluation.md) |
| S-4.13 | Replace the prose profile claims with measurements — M | `PARTIAL` | [phase-4-memory-and-evaluation.md](plan-sets/set-S/phase-4-memory-and-evaluation.md) |
| S-4.14 | The live session that has never happened — M | `BLOCKED:HUMAN` | [phase-4-memory-and-evaluation.md](plan-sets/set-S/phase-4-memory-and-evaluation.md) |
| S-5.9 | A grammar for the small tiers — M | `PARTIAL` | [phase-5-strategy-language.md](plan-sets/set-S/phase-5-strategy-language.md) |
| S-5.11 | `suggest_moves` — S | `PARTIAL` | [phase-5-strategy-language.md](plan-sets/set-S/phase-5-strategy-language.md) |
| S-5.15 | Intent survives, and the preview says which clause blocked — M | `PARTIAL` | [phase-5-strategy-language.md](plan-sets/set-S/phase-5-strategy-language.md) |
| S-6.6 | The controls row and the keyboard map — M | `PARTIAL` | [phase-6-research-desk.md](plan-sets/set-S/phase-6-research-desk.md) |
| S-6.11 | Right panel — Tools — M | `PARTIAL` | [phase-6-research-desk.md](plan-sets/set-S/phase-6-research-desk.md) |
| S-6.12 | Right panel — Cost — M | `PARTIAL` | [phase-6-research-desk.md](plan-sets/set-S/phase-6-research-desk.md) |
| S-6.16 | Run detail and the replay scrubber — L | `PARTIAL` | [phase-6-research-desk.md](plan-sets/set-S/phase-6-research-desk.md) |
| S-8.3 | The capital workflows — L | `PARTIAL` | [phase-8-integration.md](plan-sets/set-S/phase-8-integration.md) |
| S-8.9 | Close the set — M | `PARTIAL` | [phase-8-integration.md](plan-sets/set-S/phase-8-integration.md) |
| S-9.10 | Utility attribution — L | `PARTIAL` | [phase-9-context-as-retrieval.md](plan-sets/set-S/phase-9-context-as-retrieval.md) |
| S-9.12 | Fit the weights, then earn the promotion — L | `PARTIAL` | [phase-9-context-as-retrieval.md](plan-sets/set-S/phase-9-context-as-retrieval.md) |
| S-10.2 | Chat: `Lookup` forced, no ticket, and the Research offer — M | `PARTIAL` | [phase-10-lifecycle.md](plan-sets/set-S/phase-10-lifecycle.md) |
| S-10.3 | `Preregistration` and the ledger gate — L | `PARTIAL` | [phase-10-lifecycle.md](plan-sets/set-S/phase-10-lifecycle.md) |
| S-10.4 | Hypotheses name their falsifier; the platform writes the verdict — L | `PARTIAL` | [phase-10-lifecycle.md](plan-sets/set-S/phase-10-lifecycle.md) |
| S-10.8 | A `supported` verdict needs complete evidence and a reproducibility receipt — M | `PARTIAL` | [phase-10-lifecycle.md](plan-sets/set-S/phase-10-lifecycle.md) |
| S-10.10 | `compare_candidates` — SPEC §11.5 as one door — M | `PARTIAL` | [phase-10-lifecycle.md](plan-sets/set-S/phase-10-lifecycle.md) |
| S-10.11 | `optimise`: the `ResearchRun` child worker is a worker sub-run — L | `PARTIAL` | [phase-10-lifecycle.md](plan-sets/set-S/phase-10-lifecycle.md) |
| S-10.12 | Open questions are memory — M | `PARTIAL` | [phase-10-lifecycle.md](plan-sets/set-S/phase-10-lifecycle.md) |
| S-10.14 | Workflows as stage skeletons, per tier — M | `PARTIAL` | [phase-10-lifecycle.md](plan-sets/set-S/phase-10-lifecycle.md) |
| S-10.15 | The desk shows the stage, the registration, the falsifier, the proposals, the yield — L | `PARTIAL` | [phase-10-lifecycle.md](plan-sets/set-S/phase-10-lifecycle.md) |
| S-10.16 | The `judgement` suite — research decisions as a scored thing — L | `PARTIAL` | [phase-10-lifecycle.md](plan-sets/set-S/phase-10-lifecycle.md) |
| S-11.2 | Memory-fed sources: open questions, retry conditions, contradictions, stale findings, follow-ups — M | `PARTIAL` | [phase-11-continuous-research.md](plan-sets/set-S/phase-11-continuous-research.md) |
| S-11.12 | The digest — M | `PARTIAL` | [phase-11-continuous-research.md](plan-sets/set-S/phase-11-continuous-research.md) |
| S-11.13 | The desk: the toggle, the charter form, the Agenda and Charter tabs, the rail, home — L | `PARTIAL` | [phase-11-continuous-research.md](plan-sets/set-S/phase-11-continuous-research.md) |
| S-11.14 | Session tools and the API — M | `PARTIAL` | [phase-11-continuous-research.md](plan-sets/set-S/phase-11-continuous-research.md) |

### Set M — Backtest as a Tool — 22 open (17 PARTIAL, 2 NOT STARTED, 2 BLOCKED:UPSTREAM, 1 BLOCKED:HUMAN)

| task | title | status | file |
|---|---|---|---|
| M-0.2 | Prove every RunConfig field changes the run (BT-05) — M | `PARTIAL` | [phase-0-reconciliation.md](plan-sets/set-M/phase-0-reconciliation.md) |
| M-0.4 | Every run manifest states its execution policy — S | `PARTIAL` | [phase-0-reconciliation.md](plan-sets/set-M/phase-0-reconciliation.md) |
| M-1.1 | PositionSeries and its validity rules — S | `PARTIAL` | [phase-1-layer-1-position-series.md](plan-sets/set-M/phase-1-layer-1-position-series.md) |
| M-1.2 | The position-to-order planner and the series evaluator — M | `PARTIAL` | [phase-1-layer-1-position-series.md](plan-sets/set-M/phase-1-layer-1-position-series.md) |
| M-1.3 | The Gate-0 truncation test — M | `PARTIAL` | [phase-1-layer-1-position-series.md](plan-sets/set-M/phase-1-layer-1-position-series.md) |
| M-1.4 | The research_strategy SDK and runner-side harness — M | `PARTIAL` | [phase-1-layer-1-position-series.md](plan-sets/set-M/phase-1-layer-1-position-series.md) |
| M-1.5 | The research_run job, the position_series artifact and RunKind::PositionSeries — L | `PARTIAL` | [phase-1-layer-1-position-series.md](plan-sets/set-M/phase-1-layer-1-position-series.md) |
| M-1.6 | Layer 1 sweeps use the platform sampler — M | `PARTIAL` | [phase-1-layer-1-position-series.md](plan-sets/set-M/phase-1-layer-1-position-series.md) |
| M-1.8 | A tool and CLI entry for Layer 1 — S | `PARTIAL` | [phase-1-layer-1-position-series.md](plan-sets/set-M/phase-1-layer-1-position-series.md) |
| M-2.2 | EDGE spread, ADV and sigma from the instrument profile — M | `PARTIAL` | [phase-2-costs-and-outputs.md](plan-sets/set-M/phase-2-costs-and-outputs.md) |
| M-2.3 | cost_model_ref is honoured and CostSweep members differ — M | `NOT STARTED` | [phase-2-costs-and-outputs.md](plan-sets/set-M/phase-2-costs-and-outputs.md) |
| M-2.4 | sizing_ref, seed and data_snapshot are honoured — M | `NOT STARTED` | [phase-2-costs-and-outputs.md](plan-sets/set-M/phase-2-costs-and-outputs.md) |
| M-2.5 | Outputs v2: fills, positions, decisions — L | `PARTIAL` | [phase-2-costs-and-outputs.md](plan-sets/set-M/phase-2-costs-and-outputs.md) |
| M-2.6 | Annualisation from the instrument calendar — S | `PARTIAL` | [phase-2-costs-and-outputs.md](plan-sets/set-M/phase-2-costs-and-outputs.md) |
| M-2.7 | explain_trade — M | `PARTIAL` | [phase-2-costs-and-outputs.md](plan-sets/set-M/phase-2-costs-and-outputs.md) |
| M-3.1 | BatchRun: N parameter sets per data load — L | `PARTIAL` | [phase-3-throughput-and-replay.md](plan-sets/set-M/phase-3-throughput-and-replay.md) |
| M-3.3 | Trade-tick replay — L | `BLOCKED:UPSTREAM` | [phase-3-throughput-and-replay.md](plan-sets/set-M/phase-3-throughput-and-replay.md) |
| M-3.4 | Quote-tick replay and a quoted-spread cost leg — L | `BLOCKED:UPSTREAM` | [phase-3-throughput-and-replay.md](plan-sets/set-M/phase-3-throughput-and-replay.md) |
| M-4.1 | The strategy_reconciliation record — M | `PARTIAL` | [phase-4-research-to-deploy.md](plan-sets/set-M/phase-4-research-to-deploy.md) |
| M-4.2 | The reconciliation comparator — M | `PARTIAL` | [phase-4-research-to-deploy.md](plan-sets/set-M/phase-4-research-to-deploy.md) |
| M-4.3 | The market_simulator reference differential — L | `BLOCKED:HUMAN` | [phase-4-research-to-deploy.md](plan-sets/set-M/phase-4-research-to-deploy.md) |
| M-4.4 | A Layer 2 version of a Layer 1 idea cannot pass Gate 3 unreconciled — M | `PARTIAL` | [phase-4-research-to-deploy.md](plan-sets/set-M/phase-4-research-to-deploy.md) |

### Set N — AI Model Studio — 23 open (20 PARTIAL, 3 NOT STARTED)

| task | title | status | file |
|---|---|---|---|
| N-1.6 | Specification counting — M | `PARTIAL` | [phase-1-dataset-declaration.md](plan-sets/set-N/phase-1-dataset-declaration.md) |
| N-1.7 | Sampling modes: CUSUM and strategy events — L | `PARTIAL` | [phase-1-dataset-declaration.md](plan-sets/set-N/phase-1-dataset-declaration.md) |
| N-2.5 | Sequence models consume real windows — L | `PARTIAL` | [phase-2-families.md](plan-sets/set-N/phase-2-families.md) |
| N-2.6 | The sequence zoo: TCN, DLinear, N-HiTS, PatchTST, TFT, TSMixer — L | `PARTIAL` | [phase-2-families.md](plan-sets/set-N/phase-2-families.md) |
| N-2.8 | Foundation baselines with contamination metadata — M | `PARTIAL` | [phase-2-families.md](plan-sets/set-N/phase-2-families.md) |
| N-2.9 | The bring-your-own-model contract — L | `PARTIAL` | [phase-2-families.md](plan-sets/set-N/phase-2-families.md) |
| N-3.1 | The scoring additions — L | `PARTIAL` | [phase-3-scoring.md](plan-sets/set-N/phase-3-scoring.md) |
| N-3.2 | The hard usage rules as a verdict type — M | `PARTIAL` | [phase-3-scoring.md](plan-sets/set-N/phase-3-scoring.md) |
| N-3.3 | Purged MDA and clustered feature importance — M | `PARTIAL` | [phase-3-scoring.md](plan-sets/set-N/phase-3-scoring.md) |
| N-3.4 | The vol-timed momentum baseline — S | `PARTIAL` | [phase-3-scoring.md](plan-sets/set-N/phase-3-scoring.md) |
| N-3.5 | A scorecard always carries its baselines — M | `NOT STARTED` | [phase-3-scoring.md](plan-sets/set-N/phase-3-scoring.md) |
| N-3.6 | The validator enforces the usage rules at the Gate 3 binding — M | `PARTIAL` | [phase-3-scoring.md](plan-sets/set-N/phase-3-scoring.md) |
| N-4.4 | The meta-labeller — M | `NOT STARTED` | [phase-4-calibration-serving-ops.md](plan-sets/set-N/phase-4-calibration-serving-ops.md) |
| N-4.6 | Model storage columns and the series link tables — S | `NOT STARTED` | [phase-4-calibration-serving-ops.md](plan-sets/set-N/phase-4-calibration-serving-ops.md) |
| N-5.1 | Routing, navigation and the data layer — M | `PARTIAL` | [phase-5-studio-frontend.md](plan-sets/set-N/phase-5-studio-frontend.md) |
| N-5.2 | Command Center — M | `PARTIAL` | [phase-5-studio-frontend.md](plan-sets/set-N/phase-5-studio-frontend.md) |
| N-5.3 | Create wizard — M | `PARTIAL` | [phase-5-studio-frontend.md](plan-sets/set-N/phase-5-studio-frontend.md) |
| N-5.4 | Cockpit shell and Overview — M | `PARTIAL` | [phase-5-studio-frontend.md](plan-sets/set-N/phase-5-studio-frontend.md) |
| N-5.5 | Training console — L | `PARTIAL` | [phase-5-studio-frontend.md](plan-sets/set-N/phase-5-studio-frontend.md) |
| N-5.6 | Test lab — M | `PARTIAL` | [phase-5-studio-frontend.md](plan-sets/set-N/phase-5-studio-frontend.md) |
| N-5.7 | Evaluations arena — L | `PARTIAL` | [phase-5-studio-frontend.md](plan-sets/set-N/phase-5-studio-frontend.md) |
| N-5.8 | Version timeline, promotion gate and deployments — M | `PARTIAL` | [phase-5-studio-frontend.md](plan-sets/set-N/phase-5-studio-frontend.md) |
| N-5.9 | Lineage graph — M | `PARTIAL` | [phase-5-studio-frontend.md](plan-sets/set-N/phase-5-studio-frontend.md) |

### Set Q — Research Tools and EV Extensions — 9 open (5 PARTIAL, 3 BLOCKED:UPSTREAM, 1 NOT STARTED)

| task | title | status | file |
|---|---|---|---|
| Q-1.6 | `compare_to_random`: the agent against a matched-budget random search — M | `BLOCKED:UPSTREAM` | [phase-1-the-section-8-tools.md](plan-sets/set-Q/phase-1-the-section-8-tools.md) |
| Q-1.8 | `run_robustness_suite`: cost ladder, delay, sub-periods, neighbourhood, perturbations, regime — L | `PARTIAL` | [phase-1-the-section-8-tools.md](plan-sets/set-Q/phase-1-the-section-8-tools.md) |
| Q-1.9 | `compare_candidates` — S | `PARTIAL` | [phase-1-the-section-8-tools.md](plan-sets/set-Q/phase-1-the-section-8-tools.md) |
| Q-1.11 | `factor_attribution` — S | `PARTIAL` | [phase-1-the-section-8-tools.md](plan-sets/set-Q/phase-1-the-section-8-tools.md) |
| Q-2.3 | Specification count, exploration ledger and `post_hoc` reported (EV-03) — S | `PARTIAL` | [phase-2-ev-extensions.md](plan-sets/set-Q/phase-2-ev-extensions.md) |
| Q-2.5 | Dossiers carry an expected live haircut (T18) — S | `PARTIAL` | [phase-2-ev-extensions.md](plan-sets/set-Q/phase-2-ev-extensions.md) |
| Q-2.6 | Order-flow features before price-only features (T12) — L | `BLOCKED:UPSTREAM` | [phase-2-ev-extensions.md](plan-sets/set-Q/phase-2-ev-extensions.md) |
| Q-2.7 | `basis_report` and never-gross-of-cost carry (T15) — M | `BLOCKED:UPSTREAM` | [phase-2-ev-extensions.md](plan-sets/set-Q/phase-2-ev-extensions.md) |
| Q-2.8 | Hypothesis registration is required by `create_experiment` (EV-02) — S | `NOT STARTED` | [phase-2-ev-extensions.md](plan-sets/set-Q/phase-2-ev-extensions.md) |

### Set R-a — Skills Registry — 10 open (9 PARTIAL, 1 BLOCKED:HUMAN)

| task | title | status | file |
|---|---|---|---|
| RA-2.2 | The resident set: at most 40, deterministic, frozen per project version — M | `PARTIAL` | [phase-2-materialiser.md](plan-sets/set-R-a/phase-2-materialiser.md) |
| RA-2.5 | Verify each bundle's hash and signature — M | `BLOCKED:HUMAN` | [phase-2-materialiser.md](plan-sets/set-R-a/phase-2-materialiser.md) |
| RA-2.6 | Mid-session revocation as a notice — M | `PARTIAL` | [phase-2-materialiser.md](plan-sets/set-R-a/phase-2-materialiser.md) |
| RA-3.3 | The `skill_verify` job — L | `PARTIAL` | [phase-3-propose-and-verify.md](plan-sets/set-R-a/phase-3-propose-and-verify.md) |
| RA-3.4 | Replay verification: with and without the skill — L | `PARTIAL` | [phase-3-propose-and-verify.md](plan-sets/set-R-a/phase-3-propose-and-verify.md) |
| RA-3.6 | The savings condition (T5) — S | `PARTIAL` | [phase-3-propose-and-verify.md](plan-sets/set-R-a/phase-3-propose-and-verify.md) |
| RA-3.7 | Proposal triggers T1 to T4 and the single Stop-hook nudge — M | `PARTIAL` | [phase-3-propose-and-verify.md](plan-sets/set-R-a/phase-3-propose-and-verify.md) |
| RA-3.8 | A skill bill of materials per run — M | `PARTIAL` | [phase-3-propose-and-verify.md](plan-sets/set-R-a/phase-3-propose-and-verify.md) |
| RA-4.1 | `tbot skills search` over full bodies — M | `PARTIAL` | [phase-4-search-ui-procedures.md](plan-sets/set-R-a/phase-4-search-ui-procedures.md) |
| RA-4.3 | UI-06: the Skills page — L | `PARTIAL` | [phase-4-search-ui-procedures.md](plan-sets/set-R-a/phase-4-search-ui-procedures.md) |

### Set R-b — Knowledge and Learned Tiers — 20 open (14 PARTIAL, 6 BLOCKED:UPSTREAM)

| task | title | status | file |
|---|---|---|---|
| RB-1.1 | Retrieval filters by metadata first and shows staleness and capability flags (KM-02) — S | `PARTIAL` | [phase-1-knowledge.md](plan-sets/set-R-b/phase-1-knowledge.md) |
| RB-1.5 | Embeddings are local and carry model and dimension (KM-06) — S | `PARTIAL` | [phase-1-knowledge.md](plan-sets/set-R-b/phase-1-knowledge.md) |
| RB-1.6 | Hybrid retrieval store (KM-07): decide Milvus or pgvector — L | `PARTIAL` | [phase-1-knowledge.md](plan-sets/set-R-b/phase-1-knowledge.md) |
| RB-1.9 | Untrusted-derived memory is tagged `tainted` (KM-10) — M | `PARTIAL` | [phase-1-knowledge.md](plan-sets/set-R-b/phase-1-knowledge.md) |
| RB-2.4 | Skill A/B evaluation (SK-16) — L | `PARTIAL` | [phase-2-skill-promotion-and-evaluation.md](plan-sets/set-R-b/phase-2-skill-promotion-and-evaluation.md) |
| RB-2.5 | The library is re-evaluated before a model upgrade becomes the default (SK-17) — M | `PARTIAL` | [phase-2-skill-promotion-and-evaluation.md](plan-sets/set-R-b/phase-2-skill-promotion-and-evaluation.md) |
| RB-3.1 | A matched-budget shadow random arm per campaign — L | `PARTIAL` | [phase-3-random-arm.md](plan-sets/set-R-b/phase-3-random-arm.md) |
| RB-4.2 | Statistical jump model fit and persisted `regime_labels` — L | `PARTIAL` | [phase-4-regime-engine.md](plan-sets/set-R-b/phase-4-regime-engine.md) |
| RB-4.3 | `get_regime_profile` and `get_regime_prior` tools — M | `PARTIAL` | [phase-4-regime-engine.md](plan-sets/set-R-b/phase-4-regime-engine.md) |
| RB-4.4 | `regime_strategy_prior` updates — M | `PARTIAL` | [phase-4-regime-engine.md](plan-sets/set-R-b/phase-4-regime-engine.md) |
| RB-4.5 | The `add_regime_gate` move — M | `PARTIAL` | [phase-4-regime-engine.md](plan-sets/set-R-b/phase-4-regime-engine.md) |
| RB-4.6 | The learned regime tier: HMM/HSMM with BOCPD — L | `PARTIAL` | [phase-4-regime-engine.md](plan-sets/set-R-b/phase-4-regime-engine.md) |
| RB-5.1 | M3 learning-curve extrapolator — L | `BLOCKED:UPSTREAM` | [phase-5-learned-tiers-and-campaigns.md](plan-sets/set-R-b/phase-5-learned-tiers-and-campaigns.md) |
| RB-5.2 | M4 config-to-outcome surrogate — L | `BLOCKED:UPSTREAM` | [phase-5-learned-tiers-and-campaigns.md](plan-sets/set-R-b/phase-5-learned-tiers-and-campaigns.md) |
| RB-5.3 | M5 gate pre-screener — L | `BLOCKED:UPSTREAM` | [phase-5-learned-tiers-and-campaigns.md](plan-sets/set-R-b/phase-5-learned-tiers-and-campaigns.md) |
| RB-5.4 | M8 family recommender — L | `BLOCKED:UPSTREAM` | [phase-5-learned-tiers-and-campaigns.md](plan-sets/set-R-b/phase-5-learned-tiers-and-campaigns.md) |
| RB-5.5 | M9 proposal ranker — L | `BLOCKED:UPSTREAM` | [phase-5-learned-tiers-and-campaigns.md](plan-sets/set-R-b/phase-5-learned-tiers-and-campaigns.md) |
| RB-5.6 | M13 step critic — L | `BLOCKED:UPSTREAM` | [phase-5-learned-tiers-and-campaigns.md](plan-sets/set-R-b/phase-5-learned-tiers-and-campaigns.md) |
| RB-5.7 | FEAT-003 Phase 2: islands, feature map and bandit allocation — L | `PARTIAL` | [phase-5-learned-tiers-and-campaigns.md](plan-sets/set-R-b/phase-5-learned-tiers-and-campaigns.md) |
| RB-5.8 | FEAT-003 Phase 4: multi-instrument campaigns, refit scheduling and calibration-drift alerts — L | `PARTIAL` | [phase-5-learned-tiers-and-campaigns.md](plan-sets/set-R-b/phase-5-learned-tiers-and-campaigns.md) |

---

## Part B — older sets (no per-task markers; status taken from the MASTER and phase files)

The plan files were brought up to date by the 2026-10-03 PROD, LEGACY and UI tracks, so this part is
much shorter than the 2026-10-03 snapshot. Check the code before starting any item marked `verify`.

### Set E — Production readiness (22 of 30 done; MASTER re-verified 2026-10-03, track PROD)

Closed since the last snapshot: E-1.1 to E-1.6 (token hash, `/auth`, session extractor, owner-scoped
automations, no `dev-local`, nil-user rows dropped: migrations 0150 and 0151), E-2.1 to E-2.3, E-3.1,
E-3.2, E-3.3, E-3.6, E-4.1, E-5.3, E-5.4, E-5.5, and the untracked findings #3 (order audit, migration 0152),
#4 (kill switch persists) and #9 (inbound side no longer fails open on any of five venues).

| item | title | status | what remains |
|---|---|---|---|
| E-1.7 | Flip the loopback bind guard to "auth hardened" | `PARTIAL` | the guard is wired in `main.rs`; the clean `/security-review` is a **human gate**. Until then the MCP loopback bind (Set F) stays as is |
| E-3.4 | 0x on-chain submit and receipt poll | `BLOCKED:HUMAN` | real money. `submit` now refuses instead of fabricating an order id |
| E-3.5 | Coinbase live broker | `PARTIAL` | built up to the signed request; live submit is `BLOCKED:HUMAN` (needs a security review) |
| E-4.2 | `PercentOfBalance` sizing | superseded by Set U | the Set U engine plans `units/notional/equity_fraction/risk/vol_target`; Set D 3.5 recorded it done. Close the Set E block |
| E-4.3 | Scanner `/surface` endpoint and universe feed | `OPEN` | `evaluate_universe_pipeline` is still called only from tests (overlaps U-6) |
| E-4.4 | v1.5 format: exit nodes and AND/OR | superseded by Set U | v3 graphs carry exits; the v1 builder drop-with-a-warning is the remaining symptom |
| E-4.5 | RiskUnit sizing | deferred | by design |
| E-5.1 | Prometheus metrics registry and `/metrics` | `PARTIAL` | registry, loopback exporter and execution call sites done; the exporter start is now in `main.rs` (line 60, verified 2026-10-04); the **pipeline call sites** are still missing (`verify`) |
| E-5.2 | Parquet compaction and nightly scheduler | `PARTIAL` | compaction and scheduler built; the scheduler spawn and the archive writer are not wired (`verify` in `main.rs`) |
| E-audit-5 | Durable paper engine state | `PARTIAL` | `execution::paper::persist` is wired in `main.rs` (lines 280 and 283, verified 2026-10-04), so the "two lines" PROD asked for are in; the Set E MASTER still reads PARTIAL, re-mark it. Resting paper orders are not restored (documented in 0153) |
| E-audit-8 | Mutex-poison panics on order/fill paths | `PARTIAL` | `paper/engine.rs` fixed; `routes/strategies.rs` (`strategy_store.lock().expect`) is Set U territory |
| E-FS.1–3 | TigerGraph, Milvus embedder, config secrets and runbooks | deferred | future scope, not counted |

### Set H — AI Model Studio, phases 4–5 (folded into Set N; verdicts recorded in Set N phase 0, N-0.2)

H-4.1 to H-4.4 are replaced by U-5. Set N's phase 0 gave every remaining H row a verdict on 2026-10-05
(`set-N/phase-0-reconciliation.md`, table under N-0.2). Nothing in this table is "verify" any more.

| item | title | status | note |
|---|---|---|---|
| H-4.5 | Used-by derivation | `DONE` 2026-10-04 | JSONB containment on `strategy_definitions`, GIN index (migration 0200), a failed read is a 422. Test `used_by_lists_the_strategies_that_reference_the_model_and_only_those` (its fixture was repaired 2026-10-05 for 0140) |
| H-4.6 | Inference traces and cost/latency rollups | `DONE` 2026-10-05 | `GET .../traces` and `.../traces/rollup` over `inference_trace`; the ClickHouse trace row and a cost field are dropped by ADR-N-R2-05 (no cost data for local inference, no second consumer) |
| H-4.7 | Inference rate limiting and permissions | `DONE` 2026-10-05 | per-model sliding window in `InferenceGateway::forecast` (default 1200 a minute, traced as `rate_limited`); `require_human` on every alias-moving route; tests `rate_limit::tests` and pg `a_model_past_its_rate_limit_is_refused_and_the_refusal_is_traced` |
| H-4.8 | Nightly retrain orchestration | `DONE` 2026-10-05 for the scheduler and the due list | `RetrainScheduler` is spawned from `main.rs` and `due_models` is tested. **Train, then eval, then gate is not scheduled: re-cut as N-4.5b** (deferred, listed under Set N in Part A) |
| H-5.1 to H-5.9 | `/mlops` routing, Command Center, Create Wizard, Cockpit, Training Console, Test Lab, Evaluations Arena, Version Timeline and Promotion Gate, Lineage Graph | re-cut as **N-5.1 to N-5.9**, one for one | all nine are built `PARTIAL` in Set N phase 5 (see Part A); close them there, not here |

### Set L — Agent runtime, data API, toolbox

| item | status | note |
|---|---|---|
| Ph3-4: live agent session end to end (proxy, SDK start/resume/steer/interrupt, hooks) | never run | blocked on H-2 (frontier credential); same blocker as S-4.14 |
| Ph3-4 RT-15: `final_report` server-side validator | open | |
| Ph3-4: event bridge, `ask_user`, resume-on-restart, sub-agent definitions, skill materialisation | open | the HTTP steer/approval routes were built later |
| Ph3-4 CX-01 to CX-14 | partial | |
| Exit criteria #1/#2/#5 (restart survives resume, cost items, pure-noise claims no edge) | never run | need a live session |
| Ph1 (Set K leg C): SuiteManager submits Study members as child jobs | deferred | Studies still run in-process |
| Ph1 (Set K leg B): durable Run/Study/Experiment stores | deferred | a restart loses in-flight Study state |
| Ph2 DA §5: `quotes` and `book` endpoints | `BLOCKED:UPSTREAM` | no quote/book store until Set P's lane exists (ADR-L-DA-4); the other endpoints are built (search, trades, open_interest, option_chain, fundamentals, features) |
| Ph2: the remaining DA §5 endpoints (funding, prediction_markets, macro, universe, text) | open | `macro` and text depend on Set P (P-3.x partial) |
| Ph5: `tbot memory` (read/prefer/forget), core skills, v0 materialiser | `PARTIAL` | the materialiser verifies digests, not signatures, and ships without a caller (ADR-L-SK-1); signing is `BLOCKED:HUMAN` (RA-2.5) |
| Ph6: live eval trials (`noise`, `planted_edge`) | never run | dry-run only |
| Ph6: Artifacts and Experiments panes, rich cards, sub-agent lanes, EV-02, EV-10 | open | overlaps S-6 |
| Ph6: retire `AgentPage.tsx` | deferred | |
| Ph6 decision: eval margin 2 pp vs 5 pp | open | human money/scope decision |
| Ph0 tail: drop v1 `market_bars` | `BLOCKED:HUMAN` | still read by the boot backfill; irreversible. (`clickhouse/bars.rs` is already gone.) |

### Set C — Performance (closed; the "do" items are now done)

LEGACY track, 2026-10-03: the `ws/live.rs` per-subscriber clone (`DONE`, `Arc<LiveFrame>` with one serialisation),
the `hot_path.rs` per-tick `json!` (`DONE`), the busy-spin consumers (`DONE`, `IdleBackoff`), the dead
UUIDv5 code (`DONE`, deleted), and `lint-no-json-hotpath` in CI (`DONE`).

| item | note |
|---|---|
| C-metrics, end to end | the synchronous path is measured (p99 0.4–0.5 us; 6 allocations per tick, one above the final target of 5). Frame receipt to ring push, the cross-thread hand-off and the order-submit leg are **not measured**; they need the frame-receipt `Instant` in the collector and `strategy-runtime` |

### Set D — deferred

- **D-3.3** Tick/quote replay in backtest: `BLOCKED:UPSTREAM`. Neither engine consumes ticks; the exact SDK
  requirements for `market_simulator` are written in `set-D/phase-3.md` 3.3. Same item as FEAT-002 §6 and Set M phase 3.
- **D-0.1/0.2** Cut a real SDK tag in `market_simulator`. Currently an immutable git `rev`. (D-3.5, non-`Fixed` sizing, was closed 2026-10-03, absorbed by Set U.)

### Set G — doc hygiene (13 of 13 tasks done, 2026-10-03)

Two items remain and both are **human decisions**:
- Collapse the `docs/specs/README.MD` / `README.md` case collision. `README.MD` is the owner's no-touch file, so the
  10 dead links in it are listed by target in `set-G/MASTER.md` for whoever edits it.
- Decide the fate of the repo-root `research/` directory (stale second tree, 109 KB, nothing links to it).
  Recommendation recorded: delete it in one commit after a row-level diff of `OPEN_QUESTIONS.md`.

---

## Part C — written but tracked by no plan task

### C1. Engineering gaps found in ROADMAP / handoffs

| # | item | status | source |
|---|---|---|---|
| C1.1 | **Campaign child workers for `Study`, baseline and compare** | **built**: `main.rs` registers `StudyWorker`, `BaselineWorker` and `CompareWorker`, and `campaign_children` (10 live tests) passes on a clean job queue. Remaining: confirm the terminal rules `DIMINISHING_RETURNS`/`CONVERGED` in a live campaign, and an `EvalTask` worker (`verify`) | ROADMAP Stage 2, COMP-005 |
| C1.2 | **Resume checkpoint writer** | **built**: `jobs::worker` is the production caller of `plan_resume` (`checkpoint_resume` tests), the trainer writes mid-run checkpoints, migration 0185. Still open: only for frameworks AT-61 showed resume bit-identical, and a live interrupted-run drill has not been run | ROADMAP track 18 |
| C1.3 | **Phase-3 knowledge writers scheduled** | **built**: `InstrumentProfileWorker` is registered and `knowledge_jobs::spawn_timers` is called from `main.rs`. Not yet shown against a real embedder run | ROADMAP Stage 2 |
| C1.4 | **`regime_scopes`** | **partly settled**: scopes now live in `regime_causal.regime_scope` (ADR-P3-07), with the config key kept as a legacy hint; labelling logs "no market scopes configured" when empty. The OOS regime separation test still runs only from an ignored live test (`regime_separation_live`), so Gates 6 and 11 are unproven end to end | ROADMAP G-1 |
| C1.5 | **Insight store** (SPEC §5.6): retrieval endpoint, 4K cap, decay-on-contradiction, embedder | `PARTIAL`: the events table 0183 and the fingerprint 0184 exist; the retrieval endpoint and embedder are not shown | ROADMAP G-1b |
| C1.6 | Confirm all **16/16 gates compute end to end** on real inputs (gates 3/4/10/12 recovered 2026-09-22, never checked) | open | ROADMAP Stage 2 exit |
| C1.7 | The GOVERNOR `AskHuman` approval pause writes no `audit_event` before or after | open (`verify`: the effect is in the effect log; the audit row is not shown) | AT-64 residual |
| C1.8 | Close or re-declare the partial ML-context-pack checklist items: fingerprints/outcome-tensor writer (3.1/3.7), per-instrument cardinality budget (5.2), discovery loop ROC against null/planted worlds (E.14), trainer failure classification | open. `spec_15_tool_surface_conformance` and `inv23_no_governance_fields_in_tool_schemas` **pass again** (2026-10-05, `invariants` 139 of 139) | ROADMAP Stage 2 |
| C1.9 | Demonstrate one strategy running the whole funnel to a `paper_v1` verdict from 16 real inputs | open | ROADMAP Stage 2 exit |
| C1.10 | `search_scratchpad` as a first-class tool | open (no code found) | BS-007 HANDOFF |
| C1.11 | Durable resume of the GOVERNOR loop (pending approvals are cancelled on restart) | open; may belong to S-2 | BS-007 HANDOFF |
| C1.12 | `precapital_battery` is now demanded by the Significance gate (`enforced: true`, 2026-10-04) | done, but no experiment can pass that gate without a `precapital_battery` ticket, so a live funnel run is needed to find what it now refuses | `gates_demand_tickets` |
| C1.13 | **Default privileges give `app_role` DELETE on every new `public` table.** 0044's `ALTER DEFAULT PRIVILEGES` means a migration's narrower `GRANT` takes nothing away. 0167 fixed one table and 0246 (2026-10-05) fixed nineteen more. The class is still open: the next append-only table repeats it. Options: a revoke of the default (then every table that needs DELETE grants it explicitly), or an invariant that compares every `public` table's `app_role` privileges against its migration's `GRANT` | open, not planned | VERIFY2 finding |

### C2. Retirements no task owns

- `crates/api/src/features_compute.rs`: **decided, kept** (ADR-L-C2-1). It is the serving adapter over
  `features::runtime`, not the zero-filled subset DATA-006 meant to delete.
- `crates/features-py` (the PyO3 wheel): **decided, not built** (ADR-L-C2-2). `pyo3` is not in the registry here and
  `reconcile_f2.py` plus the `features_f2_reconciliation` MCP tool carry the one-engine check instead.
- Retire FEAT-003's original campaign/regime code paths at the Stage 5 exit.
- Legacy storage tests reference a `ledger_events` table no migration creates (`storage/tests/ledger_append.rs`,
  `ledger_writer.rs`, `pnl_schema.rs`), and `registry_seed.rs` expects 8 asset-class rows (and a `DataType` count) where
  migration 0006 seeds 6 and 16. Delete or rewrite them, together with `storage::ledger::LedgerWriter`, which writes to the missing table.

### C3. Reserved sets: now planned

Sets **M**, **N**, **Q**, **R-a** and **R-b** have MASTER and phase files and are counted in Part A. Still in no plan
file: AGENT-006 §5.10 LLM text features and `llm_contamination_check`; COMP-004 search backend and retention;
ML pack 0.23 and phase 6 (asset encoder, ANN, multi-node).

### C4. Product surfaces from the old queue (rewritten for the Rust/React stack)

- **FB-RX-002**: SSE/WS live bar stream into the asset chart (`verify`)
- **FB-RX-003**: execution-quality dashboard. `ExecutionQualityPage.tsx` and `routes/exec_quality.rs` exist (`verify` against the spec)
- **FB-RX-004**: governance page. `GovernancePage.tsx` exists; the release evidence, config diff and rollback viewer are `verify`
- **FB-RX-005**: alert inbox with ack/dismiss. `AlertsPage.tsx`, `routes/alerts_inbox.rs` and migrations 0158/0167 exist
- **FB-N1**: a second live venue (Kraken spot), per-venue risk caps, venue-routing UI
- **FB-N3**: portfolio optimiser objectives (max-Sharpe, min-variance, risk parity) on top of U-6's `allocation::rebalance`
- **Q-8**: raw-event retention policy

### C5. Conflicts and doc fixes

- **SLv2 (S-5) vs SLv3 (U-7.1)**: both are built (S-5 is 12 of 15 done, U-7 is 6 of 7). Which text language wins is still
  not written down in either file. Decide before more work lands in either.
- Stale claims: ROADMAP, Set T MASTER and `open-questions.md` still assert Alpaca paper in places (reversed by ADR-0048).
- `special-plans/RESTART_DATA_HEALTH_AND_FORECASTER_PLAN.MD` and the open QUEUE_STACK rows (FB-FCST-*, FB-RX-001, FB-EXEC-*)
  target the deleted Python tree. Mark them superseded.
- Pre-existing failures the Execution Board said "do not chase": four `storage` live suites, `check-money-f64` hits (47),
  `cargo fmt --check` repo-wide, CRLF renormalisation.
- Test hygiene found 2026-10-05: a scratch-database helper that applies migrations without a `_sqlx_migrations` table silently loses 0044 and every grant after it (`session_driver_veto_pg.rs` did; the stand-in table is `pg_self_monitor.rs`'s); `monitor`'s `fail_closed_drill` shares one lease table across tests and must run with `--test-threads=1` (its header says so; a default `cargo test -p monitor` fails six of its sixteen); `cargo test -p api` with `-j 6` runs the paging file out (`os error 1455`), use `-j 1`. Ignored live tests that cannot run on the 5433 server (no pgvector, no inference sidecar on 8002): `regime_separation_live`, `live_sidecar_v2` (3), `contracts::pg_tests::a_transform_is_registered_typechecked_and_callable`.
- Test hygiene found 2026-10-04: several `pg_*` and `campaign_children` suites assume an **empty job queue** and a **private
  database**, so a shared dev database makes them fail on each other's rows. `backtest/tests/pg_layer_reconciliation.rs`
  must run on a database *without* `strategy_revision` (it creates a stand-in).

### C6. Human gates (not code)

| # | what |
|---|---|
| H-1 | Deploy one hand-authored strategy to paper (T-0.4). This starts the evidence clock. See `docs/operations/forward-clock-runbook.md` |
| H-2 | Frontier provider credential (S-4.14, Set L phases 3–4) |
| H-4 / Q-4 | Capital and liability decision (Stage 6). The checklist is `docs/governance/capital-gate.md` |
| H-6 | Equities/futures onboarding decision |
| — | `/security-review` for E-1.7; 0x and Coinbase live submit (E-3.4, E-3.5); FRED key (P-3.1) and Alpaca news credentials (P-3.3); RA-2.5 bundle signing; Kraken archive files (P-1.3); the v1 `market_bars` drop |
| — | Eval margin 2 pp vs 5 pp (Set L); the fate of root `research/` and the `docs/specs/README.MD` collision (Set G) |
