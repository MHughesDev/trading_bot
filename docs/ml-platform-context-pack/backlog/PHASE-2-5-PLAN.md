# Phases 2–5 — the plan, re-scoped against the stack that exists

**Written 2026-09-14.** Normative for Phases 2–5. Every item below carries one of four decisions:

| Tag | Meaning |
|---|---|
| **BUILD** | Build as the spec says. |
| **BUILD-AS** | Build the spec's *requirement* through a different mechanism than the spec names, because the named one is not in this stack. The requirement is unchanged; the ADR says why. |
| **N/A** | The item names infrastructure this platform does not run and the requirement it carries is satisfied elsewhere or is vacuous. A revisit trigger is stated. |
| **DEFER** | Real, wanted, and correctly sequenced *after* a stated trigger. Not optional. |

Authority: `INVARIANTS.md` > `spec/SPEC.md` (as amended, Appendix C) > this plan > `reference/*`. Where this plan and the checklist disagree, the checklist has been updated to match. Each decision's reasoning is an ADR row in `decisions/ADR-INDEX.md` (P2-04 … P5-03).

## 0. The three facts this plan is shaped by

1. **The stack is one box.** Docker Compose (Postgres, ClickHouse, NATS, Redis, MinIO), a Rust platform binary, a Python trainer sidecar, one GPU (GTX 1080 Ti now; RTX 3090 is the design target per BS-007 D-18). No Kubernetes, no Temporal, no Ray, no Iceberg, no MLflow. The spec's §6/§8 infrastructure table was written for a fleet. The *requirements* those systems carry are real; the systems are not. Every N/A below names the requirement that survives and where it now lives.
2. **The durable job service already exists** (`crates/jobs`, ADR-0030, COMP-005): Postgres-backed, leased, idempotent by manifest hash, event-published over NATS→SSE, fair-share queued, budgeted at submission, **and it registers the trial inside the submission transaction**. It is the control and execution plane. Phase 2 extends it; it does not replace it.
3. **A parallel agent design exists and is authoritative for the agent.** BS-007 / AGENT-001…004 / FEAT-003 define the harness (Claude Agent SDK in a container), the toolbox (`tbot`, AGENT-002), the campaign loop (FEAT-003 §8) and the evaluation standard (BACKTEST_SUITE_CORE_SPEC v2). This pack's §15 is reconciled *into* those, not built beside them (ADR-P2-19).

Scale reality that gates Phase 4: the ledger will hold hundreds to low thousands of trials in year one, over tens of instruments, one tenant. The reference's learned-tier thresholds (M4 ≈ 1.5–5k runs over ≥ 30 tasks; M5 ≈ 500–2k gated candidates with ≥ 100 passes; M8 ≥ 100 tasks) will not be met for a long time. The cold-start ladder (§13.1) is therefore not a fallback here — it is the product for the foreseeable future, and it is built first.

---

## Phase 2 — the training system

### 2.1 Campaign workflow · **BUILD-AS** (ADR-P2-04) — *DEFINE done; the workflow is event-sourced on the job service, not Temporal*

A campaign is a **fold over an append-only event log**, driven by a job.

- **State** = fold of `mlops.campaign_event` (exists: `define … halted`). No mutable status column. Resumption is replay of the fold; there is nothing else to resume.
- **Driver** = `JobKind::Campaign` (new) on the job service: leased, heart-beaten, re-queued on worker loss. Each phase's work is a **child job** (`Study`, `GateAdvance`, `Train`, `PostHoc`) with an idempotency key `hash(campaign_id, phase, seq)`. On resume the driver folds the events, finds the child jobs that are terminal, and continues from the first that is not. This is durable execution at the only granularity we need — the phase — with the child job as the exactly-once side effect. (Reference `01-mlops-infra.md` §1.1(b): a task queue plus explicit checkpoints is sufficient when the steps are coarse; ours are.)
- **Phase semantics** (§10): `DEFINE` (done: `define_campaign`) → `BASELINE` (run the benchmark once, through the ledger) → `DIAGNOSE` (DiagnosticBundle over the baseline) → `HYPOTHESIZE` (the agent, or a human, proposes; a `propose` decision is logged with its candidate set) → `EXPERIMENT` (the dispatcher draws; child Studies) → `COMPARE` (§11.5 protocol, 2.12) → `GATE` (2.14) → `PRUNE` / `REALLOCATE` (decisions logged with propensity) → loop, or terminal.
- **Terminal rules**, all mechanical: `BUDGET_EXHAUSTED` from the job service's budget check; `DIMINISHING_RETURNS` when `P(any remaining candidate > incumbent + delta_practical) < 0.05` over the last 20 % of trials (the posterior comes from the comparison protocol's confidence sequence, 2.12); `CONVERGED` when the incumbent has been unchanged for `max(20, 0.25·trials)` trials **and** the diminishing-returns test agrees; `HALTED` only by a human, logged.
- **Principal attribution** (§8's "non-spoofable agent X for user Y"): the job service stamps `SubmittedBy` from the auth token scope (ADR-0025); the agent never writes `on_behalf_of`. That is the property Temporal's feature was wanted for, and it already holds.
- **FEAT-003 reconciliation:** FEAT-003 §8's campaign loop (islands, bandit, moves, curate) is the *agent's* strategy for `HYPOTHESIZE`; it runs inside the container and calls the platform's `EXPERIMENT` through the dispatcher. The platform owns the phase log, the budget, the floor, the gates. The agent owns what to try. FEAT-003's `research_campaigns` table is superseded by `mlops.campaign` + `campaign_event`; its `research_ledger` by `mlops.decision` + `insight`.
- **Revisit trigger for Temporal:** campaign drivers needing more than one machine, sub-second saga semantics between steps, or the platform moving to Kubernetes at all.

Acceptance: a campaign killed mid-`EXPERIMENT` and restarted resumes at the same child job, spends no second trial, and its fold is identical before and after (AT-59).

### 2.2 Job state machine incl. DEDUPLICATED · **BUILD-AS** (ADR-P2-06) — *one mapping, not a second machine*

There are two state machines and both stay: the **trial** machine (ledger, §9 semantics, exists) and the **job** machine (COMP-005 §5, exists). The work is the documented, tested mapping and one repair.

| §9 state | Job service | Ledger |
|---|---|---|
| REGISTERED | trial registered in the submit transaction | `registered` |
| QUEUED | `queued` | — |
| REJECTED | submission refused (`422`/`409`) | `failed`, reason from the refusal |
| DEDUPLICATED | manifest-hash hit (exists) | `deduplicated` naming the prior trial (exists) |
| PROVISION | `leased` | — |
| RUNNING | `running` | `running` |
| EVALUATE / GATED | child jobs of a campaign | `evaluating`, `gated` |
| PAUSED | `paused` (awaiting approval) | — (not a trial state) |
| PREEMPTED → RECOVERING | lease expiry → re-queue, `attempts += 1` | same trial, no new id |
| COMPLETED_PASS / FAIL | `succeeded` | `completed_pass` / `completed_fail` |
| FAILED(reason) | `failed` with `JobError.code` | `failed` + `TerminalReason` + `censoring` |

**The repair:** `settle_trial` today maps a job failure to a `TerminalReason` by substring-matching the error message. That becomes a typed mapping: `JobError.code` is drawn from a closed set that maps 1:1 onto `TerminalReason` (`oom`, `nan_divergence`, `data_error`, `timeout`, `leakage_detected`, `budget_exceeded`, `cancelled`, `dependency_failure`, `asha_stopped`, `integrity_rejected`, `preempted_abandoned`, `gate_failed`). An unmapped code is a build error, not a fallback to `dependency_failure`. `asha_stopped` settles `right_asha`, never `failed` — this distinction feeds M3/M4/M5 and the reference calls it load-bearing.

Acceptance: AT-22 driven through the job service for every terminal path, plus AT-60 (typed reason mapping is total).

### 2.3 Ray execution / Kueue admission · **N/A** (ADR-P2-05) — *the requirement that survives: `max_gpu_hours` is mandatory, no default*

No cluster, no gang scheduling, no spot. The three things Kueue carried land here:

- **`max_gpu_hours` REQUIRED on every `Trainer`-class manifest** (`Train`, `Hpo`, `PredictSeries`). Submission without it is `422 max_gpu_hours_required`. The worker enforces it with a hard kill at the limit → `budget_exceeded`, `right_budget`. No platform default exists; a preset may *suggest* a value the user confirms (5.7).
- **Quota per tenant/project** = the job service's per-project concurrency caps and compute budgets (COMP-005 §7). Already there.
- **Priority and fairness** = the `agent | human | system` queues at 3 : 5 : 2. Already there.

Revisit trigger: a second GPU host, or any multi-node job.

### 2.4 Bit-reproducible checkpointing · **BUILD** (ADR-P2-07) — *the contract now, tested; the policy behind a measurement*

- **Checkpoint manifest contract** (extends COMP-005 §8.2 required fields): `rng_state{python,numpy,torch_cpu,torch_cuda[]}`, `optimizer_state_ref`, `lr_scheduler_state_ref`, `dataloader_position`, `amp_scaler_state`, `step`, `code_hash`, `image_digest`, `dataset_id`. For GBDT: `boosting_round`, `seed`, and the determinism flags in effect (LightGBM `deterministic=true, force_row_wise=true`; XGBoost `seed`, single-thread for the reproducibility test). The artifact registry **refuses** a checkpoint manifest missing any field — a partial checkpoint is the classic silent-divergence bug and it is not allowed to exist.
- **Reproducibility is tested, not assumed** (AT-61): an eval task trains `n` steps, resumes from the checkpoint at `n−2`, and asserts bit-identical weights and metrics against the uninterrupted run, per framework. A framework that cannot pass is marked `checkpoint_resume: unsupported` in its adapter and 2.5 falls back to restart-under-the-same-trial for it.
- **Cadence:** proactive, on a timer, `interval ≈ √(2·checkpoint_cost·MTBF)`; asynchronous upload (separate thread) so the GPU never stalls on I/O.

### 2.5 Preempt → recover → resume without a new trial_id · **BUILD** (ADR-P2-07)

- Lost worker → the job service re-queues the same job (exists). The worker resumes from the latest checkpoint whose manifest validates; if none exists it restarts from step 0 **under the same trial** — the trial is the look, not the process.
- A checkpoint that fails validation at resume → `checkpoint_invalid` → `failed`, `preempted_abandoned`. Never "resume from something else wearing the same id".
- **Freeze-thaw (pause at a rung boundary) is off by default** and turned on per framework only when an eval task measures resume cost < 15 % of rung duration (§11.3). Until then stopping is terminal and right-censored.

### 2.6 Searcher auto-selection from (B, D, P) · **BUILD-AS** (ADR-P2-08) — *the table as a pure rule; only the arms this box can run*

`select_searcher(B, D, P, space) → Searcher`, logged on the campaign as a `propose` decision at `decision_tier = rule`. Arms:

| Condition | Searcher | Where |
|---|---|---|
| B < 10 | **portfolio replay** — no search | from the Tier-2 reference portfolio (3.6); until 3.6 exists, the FEAT-003 archetype defaults |
| 10 ≤ B < 30 | PriorBand-style: prior-weighted random over the declared ranges + the exploration floor | Rust, `crates/research` sweep engine |
| 30 ≤ B ≤ 200, mixed/conditional | **TPE** | Rust-native TPE in `crates/research` (strategy parameters are few, mixed, and the LLM-proposes/optimizer-chooses invariant lives there) |
| 30 ≤ B ≤ 200, continuous, D > 3 | GP-BO with the √D lengthscale prior (2.7) | Python sidecar (`hpo.py`, BoTorch) — model hyper-parameters only; **DEFER** for strategy parameters until a measured case needs it |
| B > 200 | CMA-ES / DEHB | **N/A** at this scale (a campaign on one box does not reach 200 full-train equivalents); revisit trigger: a campaign whose `max_trials` exceeds 200 |
| P ≥ 8 | + ASHA underneath | 2.8 |
| always | + the ≥ 5 % uniform arm | the dispatcher (2.9, built) |

### 2.7 √D-scaled LogNormal lengthscale prior · **BUILD** (with the GP-BO arm)
`ℓ ~ LogNormal(μ₀ + log(D)/2, σ₀)`. One line inside the sidecar's GP-BO; nothing to decide beyond "as specified". Trust regions and embeddings are not a first move.

### 2.8 ASHA hardening — all five · **BUILD** (ADR-P2-09) — *rungs are the fidelity ladder; the mitigations are not switches*

ASHA applies to two things here: torch training rounds, and the **fidelity ladder** of the sweep engine (FEAT-003 §7.5: tier 0 signal series → tier 1 vectorised → tier 2 simulator). A rung is a tier. All five mitigations ship as defaults with no per-mitigation off switch:

1. Rung metric is the tier's **sealed distribution statistic** (`WorstCaseRobust` for backtests; last-3 mean for training curves) — never a single last value.
2. `grace_period` from the ledger per task family; cold start = one full rung.
3. PASHA soft ranking, ε = p90 of observed rank-swap gaps from the ledger; cold start ε = 0 (plain ASHA), recorded as `rule` tier.
4. Wilcoxon pruner when ≥ 3 replicates exist.
5. **Top-3 re-evaluated with fresh platform-held seeds before an incumbent is crowned.** The seed is the campaign's §12.7 seed (2.15). This is the one that catches most of the damage and it costs three runs.

### 2.9 Exploration floor · **DONE** (ADR-P2-02/03). One addition: the self-monitoring alarm (5.3) reads `Dispatcher::below_floor`.

### 2.10 M3-backed learning-curve stopping · **BUILD-AS** (ADR-P2-10) — *the rule tier now, the PFN later*
An `EarlyStopper` interface with `decision_tier` logged. Rule tier = **median stopping rule** (§13 day-1 heuristic). Learned tier = LC-PFN class (4.4), promoted only under 4.12. Stop rule when a posterior exists: `P(final > incumbent + delta_practical | partial) < 0.05`. Stops are terminal + `right_asha` unless 2.5's freeze-thaw measurement passed for that framework.

### 2.11 Post-hoc pipeline in fixed order · **BUILD** (ADR-P2-11)
One sidecar stage, `posthoc`, run as a child job of `COMPARE`. It is **one function with no reorder flags**: soup → Caruana greedy ensemble (with replacement) over the stored per-fold OOS predictions → calibrate on the dedicated `cal` role (Platt-on-logits / beta / quadratic; binning-based calibrators are not offered) → **closed-form threshold from the cost matrix**. For GBDT the soup step is a recorded no-op (`skipped: not_applicable`), so the order stays auditable. A tuned threshold is unrepresentable: the stage has no search over the threshold at all (AT-35).

### 2.12 Comparison protocol · **BUILD** (ADR-P2-12)
`stats::compare` → a **sealed `Comparison`** carrying `profile_id`, `n_eff`, `delta_practical`, both trial counts and every flag (`non_comparable`, `overlapping_labels_unweighted`, split overrides, `uses_backfilled_knowledge`, `legacy_ungated`). Never a bare "A wins".

- **Replicates** for strategies (deterministic given data): nuisance = data resampling (out-of-bootstrap windows over the research slice) × cost-model perturbation (±20 % of the modelled costs) × platform seed for stochastic fills, **paired identically across A and B**. Target k ≈ 29; report the achieved k.
- **Decision rule:** `P(A>B) ≥ 0.75` and `P(A>B) − CI_lower > 0.5`; **anytime-valid betting confidence sequence** on the paired difference with ROPE ± `delta_practical`: promote if `CI_lower(t) > +δ`, reject if `CI_upper(t) < +δ`, else keep sampling to `k_max`, then "practically equivalent".
- **Multiplicity:** Holm for promotion decisions; BH / e-BH for screening. The search loop's validation scores are inadmissible as estimates and the type system says so: a `StudyResult` cannot be passed where a `Comparison` is required.

### 2.13 Multiple Comparison Matrix · **BUILD** (ADR-P2-13)
The comparison view is the MCM: per pair, mean difference, win/tie/loss, Wilcoxon p as a *descriptive* divergence. No critical-difference diagram exists anywhere — a static test asserts no renderer, type or endpoint contains `critical_difference` / `cd_diagram` (AT-62).

### 2.14 The gate stack, all sixteen · **BUILD** (ADR-P2-14…18) — *eight evaluators to add, and a second profile*

Mapping of §12.3's sixteen onto the funnel that exists, and the decisions for the missing eight:

| # | Gate | Status | Decision |
|---|---|---|---|
| 1 | Pre-registration | exists (`prereg_hash`, hypothesis registry) | — |
| 2 | Leakage suite 100 % | suite built, **not wired as a gate** | wire `mlops.leakage_run` (latest, blocking = 0) as Gate 0's input |
| 3 | Cost sensitivity ≥ 3× | `CostSweep` study exists, not gated | wire: breakeven multiple from the cost ladder |
| 4 | Capacity / ADV | missing | **build**: dollar ADV₂₀ from bars (single-venue, stated as such), σ_d from bars, square-root impact `0.6·σ_d·√(Q/ADV)`; capacity-at-half-Sharpe by re-running at AUM multiples (≈ 5–8 counted runs); soft 5 % / hard 10 % ADV caps as pure checks on the trade list |
| 5 | CPCV p05 > 0 | exists (Gate 2) | — |
| 6 | Walk-forward > 0 over ≥ 3 regimes | WF exists; regime count missing | build with 11 |
| 7 | PBO < 0.20 | exists (real grid now) | threshold from the profile (built) |
| 8 | DSR ≥ 0.95 on N_eff | exists | — |
| 9 | Min length: `SR ≥ 1.5√(2 ln N_eff / y)`, ≥ 5 y, ≥ 300 events | missing | **build**; thresholds stay as they are in `strict_v1` — see the profile decision below |
| 10 | Factor attribution | missing | **build per asset class** (ADR-P2-15): crypto battery from L0 — venue-weighted market (CMKT), size (CSMB), momentum (CMOM), carry/funding when a funding lane exists; equity battery FF5+MOM+STR+BAB+QMJ from the free Ken French library when an equity source is live. `t(α) ≥ 3.0` Newey–West HAC, `R² < 0.7`, `|β_mkt| ≤ 0.3` for neutral claims. Net of costs. |
| 11 | Regime coverage | missing | **build on vol terciles now** (rule tier, ADR-P2-16), swap to M7 (3.5) later. `crisis_windows` declared in the profile (crypto: 2020-03, 2021-05, 2022-05, 2022-06, 2022-11, 2023-03, 2024-08, 2025-Q1); ≥ 2 required in OOS *when the data spans them*; ≤ 50 % PnL from one regime; worst-regime Sharpe > −0.5 |
| 12 | Perturbation / concentration | Neighborhood + plateau exist | add the `∂Sharpe/∂θ` bound and per-instrument PnL ≤ 20 % (from the per-instrument artifact, 5.2); single-instrument strategies pass trivially and the verdict says so |
| 13 | Stationary bootstrap p05 > 0 | missing | **build as a ledger statistic** (ADR-P2-17): Politis–Romano, Politis–White block length, ≥ 1000 resamples over the stored return series. No new runs; not a Study; not a trial |
| 14 | Romano–Wolf vs the family | missing | **build** (ADR-P2-17): over every trial in the campaign (INV-18 makes the series available), studentized, stationary bootstrap of the joint max, B ≥ 5000, α = 0.05. This becomes the primary family-wise control for campaigns; BHY-within-N_eff stays for screening and for non-campaign Experiments |
| 15 | Paper / shadow | missing (the BS-007 "Track", G-11) | **build** over the reconciliation crate: signal reproduction ≥ 99 %, slippage ≤ 1.5×, turnover ± 20 %, zero model-attributable rejects; kill criteria (SPRT/CUSUM vs `SR_expected`, MDD > 1.5×) **pre-registered into the campaign at deployment** |
| 16 | Capital ramp | missing | **build the registry side**: `allowed_fraction ∈ {0, .10, .25, .50, 1.0}` is a fact only the promotion path can raise; the risk gate (COMP-002) reads it. "The registry is the only path to capital." The trading side is out of the pack's scope; the interface is not |

**The profile decision (ADR-P2-14).** `strict_v1` keeps Gate 9's five-year floor. This platform's history is months old, so nothing can pass `strict_v1` for years — and that is the *correct* reading of P-01, not a bug to route around. But a strategy that cannot reach paper cannot *accumulate* the years honestly. So a second profile, **`paper_v1`**, is created (never by editing `strict_v1`): identical thresholds except Gate 9's calendar floor (`min_track_record_years` = 0, `MinBTL(N_eff)` and ≥ 300 events kept) and Gate 16 (no capital ramp — `paper_v1` cannot authorise capital at all). Live capital requires `strict_v1`. Comparisons across the two are non-comparable (built). The forward-test evidence a `paper_v1` pass accumulates is exactly the Gate 15 input `strict_v1` will later need.

**Order:** 13, 14, 11 (rule), 9, 2/3 wiring, 12 completion, 4, 10 (crypto), 15, 16.

### 2.15 Gate-hacking countermeasures · **BUILD** (ADR-P2-18)
Failures counted: done. Holdout ledger: done. **Platform-held seed:** `mlops.campaign.platform_seed` generated at DEFINE, **no SELECT grant for `agent_role`** and absent from every API type (static test, AT-63); Gates 11–13 and 2.8's re-evaluation draw from it.

### 2.16 Agent tool surface · **BUILD-AS** (ADR-P2-19) — *AGENT-002 is the catalogue; §15 is its conformance checklist*
Three tool-surface designs exist (§15, `04-agentic-ml.md` §9, AGENT-002/BS-007 14). One survives: **AGENT-002**. §15 becomes a conformance test over it:
- every compute-dispatching tool has `dry_run`, `estimate_cost`, `idempotency_key`, `response_format`, structured errors with `nearest_valid` and `suggested_fix`, self-describing truncation, slugs, and a tool-use example;
- the **not-exposed list** (thresholds, `delta_practical` after DEFINE, N_eff, deflation, holdout beyond one call, `regime_research`, exploration floor, `platform_seed`) is a static test over the catalogue (extends AT-24/27, the INV-23 tests);
- §15 tools AGENT-002 lacks are **added to AGENT-002**, never beside it: `diff_datasets`, `describe_asset` / `find_similar_assets` / `get_regime` (Phase 3), `compare_candidates` (2.12), `run_gates`, `explain_failure`, `request_promotion`, `search_insights` / `write_insight` (3.10), `estimate_cost`. `adjust_search_space` / `reallocate_budget` / `prune_branch` are FEAT-003's Move vocabulary plus the `REALLOCATE` / `PRUNE` phases.

### 2.17 Approval envelopes · **BUILD** (ADR-P2-20)
Exactly four gated actions, and the gate is a **job pause**, not a prompt (`paused (awaiting approval)` exists): (1) spend above `approval_spend_usd` — a REQUIRED DEFINE field, no default; (2) promotion to paper or live via `request_promotion` → `ApprovalRequest` (the Approvals page exists); (3) sealed-holdout access — the claim is still one-per-lineage, and `paper_v1`/`strict_v1` both require the approval; (4) tier-3 insight writes (3.10). Everything else runs free inside the envelope.

### 2.18 Hash-chained audit trail with `record_phase: pre` · **BUILD** (ADR-P2-21)
`mlops.audit_event` exists and chains. The **independent writer is the API layer**: `dispatch_tool` writes `pre` *before* the policy check and `post` after, so a denial is provable; trust level from the token scope. A static test asserts every approval-gated action has a `pre` record path (AT-64).

### 2.19 *(new)* Sample weighting in the frame contract · **BUILD** (ADR-P2-22)
The dataset builder emits `sample_weight` (`dataplane::label::uniqueness_weights`, exists) as a Parquet column; the trainer passes it to xgboost / lightgbm / torch (all accept sample weights). The training path then declares `sample_weight_method = uniqueness` truthfully, and `overlapping_labels_unweighted` clears. Closes ADR-P1-01's follow-up.

---

## Phase 3 — the knowledge plane

| Item | Decision | Design |
|---|---|---|
| **3.1** Tier-1 fingerprints + EDGE | **BUILD** (ADR-P3-01) | Computed by `JobKind::InstrumentProfile` (exists) in the sidecar: the finance block (RV at 5 horizons, vol-of-vol, skew/kurtosis, bipower + jump intensity, variance ratios 1/5/21, ACF 1/5/21, Hurst, downside/upside vol, Amihud, Kyle λ where signed volume exists else absent-with-flag, **EDGE from OHLC**, turnover/ADV, overnight/intraday split — `0`+flag for 24/7 venues, seasonality coefficients, crypto factor betas, class/venue one-hots) + `pycatch22` + a TSFEL subset. `info_class = market_public`. **Fingerprints are not strategy features**: a static test asserts the feature runtime cannot resolve an embedding dimension. If one is ever wanted as a feature it is registered as a windowed feature like any other. |
| **3.2** Crypto seasonal deflation | **BUILD** | `dataplane::crypto` profile applied before any vol feature in 3.1. |
| **3.3** `asset_embedding` | **BUILD** | `schemas/05_knowledge.sql` DDL → migration 0050. `knowledge_time` = max bar knowledge time in the window, and AT-43 is a query-layer bound. |
| **3.4** Whitening → 48-d, mutual proximity | **BUILD** (ADR-P3-05) | PCA-whiten per `embedding_version` over the market-public population; exact kNN in pgvector (compose image → `pgvector/pgvector:pg16`); mutual-proximity over the candidate set; intrinsic dimension monitored. ANN stays 6.4. |
| **3.5** Regime model, GRANT split | **BUILD** (ADR-P3-02) | Order: schemas + GRANT first (AT-42 is build-blocking and costs nothing), then the rule tier (vol terciles on daily RV), then HMM/HSMM filtered + BOCPD in the sidecar. **3 states, monthly refit**, `model_version` = fit date, `p_filtered` only. Labels statistical (`high_vol/…`), never narrative. Validation = OOS regime-conditional separation or the model is decorative. |
| **3.6** Tier-2 reference portfolio | **BUILD**, after 3.7 has outcomes | Greedy submodular coverage over the outcome matrix; start at 8 (cold start: the FEAT-003 archetype defaults), grow to ~24. The portfolio scores, not the statistics, carry the routing signal (R-03). |
| **3.7** Outcome tensor + MNAR | **BUILD the fact table now; DEFER the completion model** (ADR-P3-03) | `outcome_tensor_fact` is a view over ledger × regime × cluster from day one. The exp-family CP model with logit missingness and the `b₁` test ships when `≥ 500 trials` and `≥ 20 instruments` exist; until then the recommender's rule tier (default policy) answers and the health metric reads `not fitted`. |
| **3.8** LCB + shrink, non-bypassable | **BUILD** with 3.7 | `dr_estimate` has no SELECT grant outside the recommender and appears in no API type (AT-44 static). |
| **3.9** `b₁` health metric | with 3.7 | — |
| **3.10** Insight store | **BUILD** (ADR-P3-04) | The pack's `insight` table **is** AGENT-003's durable store for findings/tiered memory: evidence CHECK (exists in DDL), 4K-token cap server-side in the retrieval endpoint, decay on contradiction, local embeddings (D-03) over claim text only. One store, two spec names. |
| **3.11** Embedding validation | **BUILD** | An eval task: retrieval precision@k against time-held-out cluster labels; transfer lift vs a random baseline measured on the ledger; temporal stability (ARI between versions). A new `embedding_version` becomes default only on passing. |

---

## Phase 4 — internal models

**Decision (ADR-P4-01): build the ladder and every rule tier now; each learned tier sits behind an explicit ledger-size trigger.** This is what §13.1 says to ship on day one, and at this platform's scale it is the product.

| Model | Rule tier (build now) | Learned tier trigger | Scope / tier |
|---|---|---|---|
| M1 cost | analytic: rows × features × steps / hw-class throughput; residual-fit later | ≥ 200 completed runs per (flow family, hw class) | global · A |
| M2 failure | pre-flight arithmetic (VRAM vs capacity, LR envelope) + rule table | ≥ 300 runs with ≥ 50 per failure class | global · A |
| M3 learning curve | median stopping rule (2.10) | ≥ 1 000 curves, ≥ 20 % run to completion (budget 5 % of compute for the anchor) | global · A |
| M4 surrogate | the searcher's own model | ≥ 1 500 runs across ≥ 30 tasks | hierarchical · B |
| M5 gate pre-screener | monotone rule on the Gate-2/3 margin at 99 % recall | ≥ 500 gated candidates **and ≥ 100 passes** | hierarchical · B |
| M6 asset encoder | Tier 1+2 (3.1, 3.6) | Tier 3 only on a probing win (6.1) | global · A |
| M7 regime | vol terciles | HMM when 3.5's validation passes | global · A |
| M8 family recommender | global family win rates from the ledger | ≥ 100 tasks | **per-tenant · B** |
| M9 proposal ranker | surrogate mean + κ·σ (acquisition) | ≥ 3 000 ranked items in ≥ 100 groups | hierarchical · B |
| M10 leakage | built (the suite + injections) | GBDT over the injection corpus when ≥ 1 000 planted cases exist | global · A |
| M11 anomaly | hard rules (NaN/Inf, monotone loss) + robust-z + CUSUM | ≥ 300 curves | global · A |
| M12 executor | Phase 6 | unchanged | — |
| M13 step critic | rules + judge triage (AGENT-004) | ≥ 3 000 labelled steps | global · B |

Cross-cutting, all **BUILD**:
- **4.1** ladder framework + `decision_tier` on every decision (the dispatcher already logs `rule`).
- **4.11** learning-debt trigger for Tier A: retrain iff `ρ_t > c_churn/(c_churn + c_wait)`, `ρ_t` = champion's rolling error minus a challenger's on recent data. Dashboards are not a signal (R-11).
- **4.12** promotion machinery over the existing `internal_model_registry` / `promotion` / `freeze` tables: challenger vs champion on the frozen holdout under 2.12's protocol; Tier A auto-promote/auto-rollback; Tier B promotion is an approval (2.17); Tier C is an access-control boundary (AT-49, exists).
- **4.13** collapse guards: training sets are ledger *ranges* `[0, seq_max]` (accumulate, never a window); every set must contain ≥ 1 outcome whose `outcome_source ∈ {paper, live}` — the builder refuses otherwise; KL **and** Wasserstein vs the seed holdout.
- **4.14** entropy SLO (ADR-P4-03): dispatch-propensity entropy over the last window ≥ `0.5·ln k`; below floor blocks Tier-B promotion and alarms.
- **4.15** seed holdout (ADR-P4-02): the first `K = 200` trials per tenant, frozen at the moment the first internal model is trained, recorded once in `mlops.seed_holdout`; AT-50 is a trial-id set intersection.
- **4.16** freeze switch: `internal_model_freeze` exists; the trainer checks it before any internal-model train or promote.
- **4.17** private eval harness (ADR-P4-04): AGENT-004's suite **is** it. The added rule: no public benchmark score may gate a harness or tool release; the non-inferiority gate is the only gate.

---

## Phase 5 — surfaces

| Item | Decision | Design |
|---|---|---|
| **5.1** NATS → SSE, viewer-scaled | **mostly exists** | Job/research events already flow `jobs.<project>.<job>` → SSE (COMP-005 §6); training metrics use the same subjects at the 5 s rate limit. **Build:** the bounded drop-oldest telemetry queue on the trainer side (AT-55). |
| **5.2** Cardinality budget; per-instrument Parquet | **BUILD** | Static test: no metric name may embed an instrument id; per-instrument results only as artifacts (`per_instrument_pnl`, which Gate 12 needs). Budget ≤ 100k keys/tenant enforced in the metrics writer. |
| **5.3** Platform self-monitoring | **BUILD first** (ADR-P5-01) | One endpoint, one MlOps page section, every §16.2 signal. Most are computable today: exploration fraction, consistency p99, holdout call ledger, gate pass rate by profile, N_eff vs trial growth, leakage suite status. The rest (M5 ECE, policy entropy, `b₁`, corpus half-life) render as an explicit **`not fitted` / `not applicable` state** — never blank, never zero. "Iceberg tag coverage" becomes **artifact pin coverage**: every registered artifact's content hash resolves. |
| **5.4** Comparison + lineage diff + "why A won" | **BUILD** after 2.12 | The MCM, gate-by-gate margins, every flag, regime-sliced deltas, and the `simplify` ablation ledger. Attribution is arithmetic, not narrative. |
| **5.5** Gate-by-gate with trial-count context | **BUILD** with 2.14 | `FunnelView` extended to sixteen gates, `profile_id`, `N_eff`, and the counter beside every number. |
| **5.6** Agent action review timeline | **BUILD** after 2.18 | The workspace Timeline gains `audit_event` `pre`/`post` rows and `decision` rows (candidate set, propensity, exploration flag, tier). |
| **5.7** Preset / guided / expert | **BUILD last** (ADR-P5-02) | Modes are presets over the one DEFINE schema. A preset carries a *suggested* `delta_practical` and `max_gpu_hours` that the user must confirm — no preset supplies a default the validator would otherwise refuse. Guided shows the computed embargo and the leakage pre-check; expert is the same JSON through the same validator. |
| **5.8** Visual pipeline builder | **DEFER** (ADR-P5-03) | The strategy builder exists; an ML-pipeline builder (feature set → label → split → model) is deferred until a third pipeline-stage type exists. Guided mode covers the need. |

---

## Phase 6 — unchanged. Deferred by decision.

---

## Build order across phases

```
2.19 (weights) ─┐
2.14 G13/G14/G11/G9 ──► 2.14 wiring G2/G3/G12 ──► 2.14 G4/G10
2.2 mapping ──► 2.1 driver ──► 2.12/2.13 ──► 2.11 ──► 2.14 G15/G16
2.3 (max_gpu_hours) ──► 2.4/2.5 ──► 2.8 ──► 2.10 ──► 2.6/2.7
2.15 ──► 2.18 ──► 2.17 ──► 2.16 conformance
3.5 (schema+GRANT) ──► 3.1/3.2/3.3 ──► 3.4 ──► 3.11 ──► 3.10 ──► 3.7 fact ──► 3.6 ──► 3.7 model/3.8/3.9
4.1 ──► 4.15/4.16 ──► M1/M2/M11 rule tiers ──► 4.11/4.12/4.13/4.14 ──► learned tiers on trigger
5.3 ──► 5.5 ──► 5.2 ──► 5.4 ──► 5.6 ──► 5.7
```

The first thing to build is **5.3**, before any of the rest: a platform that cannot report on its own judgment cannot tell whether the rest is working.

---

## Acceptance tests added by this plan

Numbered on from AT-58 in `ACCEPTANCE-TESTS.md`: AT-59 campaign resume is a pure fold; AT-60 typed terminal-reason mapping is total; AT-61 checkpoint resume is bit-identical; AT-62 no critical-difference renderer exists; AT-63 `platform_seed` is unreadable by the agent role and absent from every API type; AT-64 every approval-gated action has a `pre` audit record; AT-65 `paper_v1` cannot raise `allowed_fraction`; AT-66 a `Trainer` manifest without `max_gpu_hours` is refused; AT-67 sample weights reach the model; AT-68 fingerprint dimensions are unresolvable as features; AT-69 self-monitoring renders `not fitted` rather than a number for an unfitted model.
