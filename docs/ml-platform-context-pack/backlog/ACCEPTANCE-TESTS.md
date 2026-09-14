# Acceptance tests

Tests that must exist and pass. Several **block the build** by design — marked ⛔. A test that only proves the happy path works is incomplete; each must also prove the *violation is rejected*.

---

## Data plane

**AT-01 ⛔ · No non-PIT read path exists.**
Static analysis: no module outside `data/pit_reader` imports the raw bar tables. Fails the build on violation. *(INV-02)*

**AT-02 · PIT read performance.**
A PIT read over a 10-year range completes within 1.2× the equivalent non-PIT scan. Proves `restatement_index` pruning works and removes the excuse for a fast path. *(§1.3)*

**AT-03 ⛔ · No adjusted-price columns.**
Scan `information_schema` for `adj_%`, `%_adjusted`, `split_adjusted%`. Any hit fails the build. *(INV-03)*

**AT-04 ⛔ · No float price columns.**
Scan `information_schema`: every column matching price semantics is `DECIMAL(38,18)`. *(INV-05)*

**AT-05 ⛔ · No symbol in any key.**
Scan constraints: `symbol` appears in no PK or FK. *(INV-04)*

**AT-06 · Corporate-action mutation test.**
Ingest a split retroactively. Assert: (a) no historical bar row changed, (b) a dataset built before the split still hashes identically, (c) a dataset built after, with a later `as_of_knowledge_time`, produces different adjusted values at read time. *(INV-03, INV-12)*

**AT-07 · Futures roll look-ahead test.**
Construct a roll rule on open interest. Assert the roll cannot fire before `decision_time`. Inject a rule that tries to use same-day OI; assert rejection. *(INV-08)*

**AT-08 · Greeks absent.**
No greek column exists; read-layer greek computation is deterministic given stored IV inputs. *(INV-07)*

**AT-09 · Reorg handling.**
Simulate a 12-block reorg. Assert orphaned rows retain `orphaned_at`, dependent trials are flagged, and a `finalized_only` dataset never saw the orphaned data. *(INV-09)*

**AT-10 ⛔ · asof nearest unreachable.**
Static analysis: no import of the underlying join function outside the wrapper; wrapper rejects `strategy='nearest'`. *(INV-10)*

**AT-11 · Staleness columns present.**
Every aligned feature emits `_age_minutes` and `_quality`. Golden-file test on a cross-asset frame spanning a holiday and a crypto weekend. *(INV-11)*

**AT-12 · Calendar version pinning.**
Change the `exchange_calendars` version; assert every affected `dataset_id` changes. *(§2, INV-12)*

## Features and datasets

**AT-13 ⛔ · Lookback enforcement.**
Register a feature declaring `lookback_bars=20` whose code reads 50. Assert registration fails. This is the only test that catches a lying feature. *(INV-13)*

**AT-14 · Dataset reproducibility.**
Rebuild a dataset from its `dataset_id` 24 hours later; assert byte-identical output. *(INV-12)*

**AT-15 ⛔ · One feature implementation.**
Static analysis: backfill and live paths resolve to the same function objects. *(INV-14)*

**AT-16 · Consistency diff populated and clean.**
`feature_consistency_diff` has rows for the last 24h; p99 absolute relative diff < 1e-9 for deterministic features; zero `code_drift` diagnoses. *(INV-14)*

**AT-17 · Embargo computation.**
For a pipeline with `horizon=30`, `max_lookback=60`, `knowledge_lag=120s`, `settlement=1`: assert `embargo_bars = 93`. Assert a user-lowered value records an override that surfaces in comparison output. *(INV-15)*

**AT-18 · Purge on t1.**
Construct overlapping labels; assert purging removes samples whose `t1` falls in the test window, not merely those whose `t0` does. *(INV-15)*

## The ledger

**AT-19 ⛔ · No trial without a ledger row.**
Attempt to dispatch compute with no `REGISTERED` trial_id, via every code path including test fixtures and admin tooling. Assert refusal in all cases. *(INV-16)*

**AT-20 ⛔ · No UPDATE on the trial table.**
Assert the application role has no UPDATE grant. Attempt an update; assert failure. Corrections must append with `supersedes`. *(INV-19)*

**AT-21 · Hash chain integrity.**
Chain verification job over the full ledger passes. Tamper with a row in a copy; assert detection. *(INV-19)*

**AT-22 · All terminal states write.**
Drive a trial into each of: completed, gate-failed, OOM, NaN, timeout, cancelled, preempted-then-abandoned, ASHA-stopped. Assert a ledger row exists with correct `censoring` in every case. *(INV-17)*

**AT-23 ⛔ · Propensity required.**
Attempt to log a decision with null propensity from a policy not marked `legacy_unlogged`. Assert rejection. *(INV-20)*

**AT-24 · Exploration floor achieved.**
Over a 500-trial synthetic campaign, assert ≥5% carry `exploration_flag=true`. Attempt to set the floor below 5% via the agent tool surface; assert the field does not exist. *(INV-21)*
*Status 2026-09-14:* the tool-schema half and the type/schema/dispatcher halves are built (ADR-P2-02); the 500-trial synthetic campaign runs once 2.1's driver exists.

**AT-25 · Dedup returns prior trial.**
Dispatch the same `config_hash` twice; assert the second returns `DEDUPLICATED` with the original `trial_id` and no compute is spent. *(§9)*

**AT-26 · Per-fold predictions and return series persisted.**
For every completed trial in a sample, assert `predictions_uri` and `returns_uri` resolve to readable artifacts with expected shapes. *(INV-18)*

## Evaluation

**AT-27 ⛔ · N_eff cannot be supplied.**
Assert no API parameter accepts a trial count. Attempt to pass one; assert rejection. *(INV-22)*

**AT-28 · N_eff counts gate failures.**
Run 100 trials, fail 90 at gates. Assert N_eff computation includes all 100 before correlation clustering. *(§12.4, §12.7)*

**AT-29 ⛔ · Gate profiles immutable.**
Attempt to UPDATE a gate threshold in `strict_v1`; assert rejection. Assert creating `strict_v2` succeeds and that comparisons spanning profiles are flagged non-comparable. *(INV-23)*

**AT-30 · Random-label test.**
Permute labels on a known pipeline; assert |Sharpe| < 0.2. **This runs nightly against the platform itself, not only per strategy.** *(§12.5)*
*Amended 2026-09-14 (ADR-P1-06):* the threshold is applied to the **t-statistic** of the mean out-of-sample return (limit 4.0), not to a Sharpe ratio, because only the t-statistic has a calibrated null across sample sizes. Built and nightly.

**AT-31 · Causal access guard.**
Run a pipeline containing a deliberate future read under the guard; assert it raises. *(§12.5)*

**AT-32 · Snapshot reproducibility.**
Re-run a 6-month-old backtest against its pinned snapshot; assert identical positions. *(§12.5)*

**AT-33 · Sealed holdout rate limit.**
Request the sealed holdout twice for one strategy lineage; assert the second returns the first result with a notice and is logged. *(§12.7)*

**AT-34 ⛔ · No scalar score column.**
Assert the outcome type has no `score`/`fitness`/`objective_value` column and that selection requires an explicit objective + constraints. *(ADR-012)*

**AT-35 · Threshold is closed-form.**
Assert threshold selection consumes the cost matrix and performs no search. A tuned threshold would be an uncounted trial. *(§11.4)*

## Tenancy

**AT-36 ⛔ · Feature firewall.**
For every global-scope model, assert every feature's `info_class` ∈ {platform_physics, methodology, market_public}. Fails the build on violation. *(INV-24)*

**AT-37 ⛔ · RLS under connection pooling.**
Run the tenant-isolation suite through PgBouncer in transaction mode, as the table-owning role. Assert isolation holds. This catches both the `FORCE ROW LEVEL SECURITY` omission and the `SET` vs `SET LOCAL` leak. *(S-2)*

**AT-38 · RLS policy composition.**
Assert adding a policy does not widen access beyond intent — policies combine with `OR`. *(S-2)*

**AT-39 ⛔ · Shared-plane credential cannot name tenant prefixes.**
Assert the shared-plane role's credential has no read path to any tenant object-storage prefix. Attempt a read; assert failure at the credential layer, not the application layer. *(§7.1)*
*Status:* deferred with checklist 0.23 — no tenant-scoped object writes exist yet. Becomes build-blocking with the first.

**AT-40 · No tenant_id one-hot in global models.**
Assert no global model's feature list contains a tenant identifier in any encoding. *(ADR-018)*

**AT-41 · M8 is per-tenant.**
Assert the strategy-family recommender is never instantiated at global scope. *(§7.4)*

## Knowledge plane

**AT-42 ⛔ · Backtest role has no grant on `regime_research`.**
Attempt to read smoothed probabilities as the backtest role; assert permission denied. *(INV-23, R-07)*

**AT-43 · Embedding knowledge_time bound.**
Request neighbors as-of a past date; assert no returned embedding has `knowledge_time > as_of`. *(§5.3)*

**AT-44 ⛔ · No raw point estimates from ledger recommendations.**
Assert the recommender returns only LCB-ranked, shrunk values. Assert no parameter disables shrinkage. *(R-06, §5.5)*

**AT-45 · Censoring handled, not dropped.**
Assert the tensor completion path consumes `censoring` and `censor_at_step`, and that dropping censored rows changes the result — proving they are used. *(§5.5)*

**AT-46 · Insight requires evidence.**
Attempt to write an insight with empty `evidence_trial_ids`; assert rejection. *(§5.6)*

**AT-47 · Memory injection cap.**
Assert retrieved memory per agent turn never exceeds the 4K-token cap, server-side. *(§5.6)*

## Internal models and retraining

**AT-48 · Cold-start ladder logged.**
Assert every decision records `decision_tier`, and that the production A/B comparing rule vs learned tier is computable from the log alone. *(§13.1)*

**AT-49 ⛔ · Tier C models cannot self-modify.**
Assert no internal model training path has write access to gate thresholds, `delta_practical`, N_eff computation, the deflation code, or position sizing. *(§14.3, INV-23)*

**AT-50 · Frozen holdout untouched.**
Assert the seed holdout slice has never appeared in any internal model's training set, verified by trial_id set intersection. *(§14.4)*

**AT-51 · Entropy floor blocks promotion.**
Drive a ranker's policy entropy below floor; assert promotion is blocked. *(§14.4)*

**AT-52 · Real-outcome floor.**
Assert every internal-model retraining set contains ground truth from paper/live performance, not backtest results alone. *(§14.4)*

**AT-53 · Freeze switch.**
Assert the global freeze halts all internal-model training and promotion within one cycle. *(§14.3)*

**AT-54 · CBPE scope.**
Assert CBPE-derived signals gate only M1/M2/M3/M11 and never M4/M5/M8/M9. *(R-12, §14.2)*

## Observability

**AT-55 · Telemetry cannot stall training.**
Fill the telemetry queue; assert the training job continues and oldest entries drop. *(§16.1)*

**AT-56 · Metric cardinality budget.**
Assert per-tenant distinct metric keys ≤ 100k and that per-instrument results route to Parquet artifacts, not metric series. *(§16.1)*

**AT-57 · Tracking store is not a deployment dependency.**
Kill the tracking store; assert in-flight training continues to completion. *(§17)*
*Amended 2026-09-14 (ADR-P2-23):* there is no MLflow; the "tracking store" is the NATS event stream and ClickHouse metric writer. The test kills NATS and ClickHouse mid-training and asserts the job completes and the ledger settles.

**AT-58 · Self-monitoring signals present.**
Assert every signal in §16.2 is computed and alarmed: M5 calibration, policy entropy, MNAR `b₁`, gate pass rate, exploration fraction, consistency p99, corpus half-life, N_eff vs trial growth, tag coverage, holdout call ledger.
*Amended 2026-09-14 (ADR-P5-01):* "tag coverage" is **artifact pin coverage** (every registered artifact's content hash resolves); a signal whose model is unfitted must render an explicit `not fitted` state, never a number (AT-69).

## Added by `PHASE-2-5-PLAN.md` (2026-09-14)

**AT-59 · Campaign resume is a pure fold.**
Kill the campaign driver mid-`EXPERIMENT`; restart. Assert the fold over `campaign_event` is identical before and after, the driver resumes at the same child job, and no second trial is registered. *(§10, ADR-P2-04)*

**AT-60 · Terminal-reason mapping is total.**
Assert every `JobError.code` maps to exactly one `TerminalReason` and that an unmapped code fails the build; assert `asha_stopped` settles `right_asha`, never `failed`. *(§9, ADR-P2-06)*

**AT-61 · Checkpoint resume is bit-identical.**
Per framework: train `n` steps; resume from the checkpoint at `n−2`; assert bit-identical weights and metrics against the uninterrupted run. A checkpoint manifest missing any contract field is refused by the artifact registry. *(§9, ADR-P2-07)*

**AT-62 ⛔ · No critical-difference renderer exists.**
Static analysis: no type, endpoint or component matches `critical_difference` / `cd_diagram`. *(§11.5, ADR-P2-13)*

**AT-63 ⛔ · The platform seed is unreadable.**
Assert `agent_role` has no SELECT grant on `campaign.platform_seed` and that no API response type carries it. Attempt a read as the agent role; assert permission denied. *(§12.7, ADR-P2-18)*

**AT-64 · Denials are provable.**
For each of the four approval-gated actions, assert an `audit_event` with `record_phase = 'pre'` is written before the policy check; deny the action; assert the `pre` record exists and no `post` record does. *(§15, ADR-P2-21)*

**AT-65 ⛔ · `paper_v1` cannot authorise capital.**
Assert a `paper_v1` pass cannot raise `allowed_fraction` above 0, and that a comparison between a `paper_v1` and a `strict_v1` verdict is flagged non-comparable. *(§12.3, ADR-P2-14)*

**AT-66 ⛔ · `max_gpu_hours` is required.**
Submit a `Trainer`-class manifest without `max_gpu_hours`; assert `422`. Submit with it; exceed it; assert the job ends `budget_exceeded` with `right_budget` censoring. *(§8, ADR-P2-05)*

**AT-67 · Sample weights reach the model.**
Assert the frame carries `sample_weight`, that every trainer adapter passes it, and that a trained model differs from one trained unweighted on a frame with overlapping labels. *(§3.4, ADR-P2-22)*

**AT-68 · Fingerprint dimensions are not features.**
Assert the feature runtime cannot resolve any `asset_embedding` dimension by name. *(§5.2, ADR-P3-01)*

**AT-69 · Unfitted signals say so.**
For each §16.2 signal whose model is unfitted, assert the endpoint returns an explicit `not_fitted` state rather than a numeric value. *(§16.2, ADR-P5-01)*
