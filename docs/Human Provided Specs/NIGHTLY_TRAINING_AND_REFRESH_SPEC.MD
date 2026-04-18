# Nightly Training and Refresh Spec
## Forecaster + RL Policy Maintenance Cycle

> **Target architecture:** For the **APEX canonical** migration and **intended** end-state system design, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`new_specs/canonical/`](new_specs/canonical/). This document remains **historical / legacy intent** for the pre-canonical pipeline where applicable; where it conflicts with canonical APEX on **future** architecture, prefer canonical specs for roadmap decisions.


Version: 1.0  
Document type: execution spec  
Scope: recurring nightly training, evaluation, and candidate refresh for the forecaster and policy model  
Purpose: define the exact nightly retraining cadence, run counts, seed counts, refresh logic, validation rules, and promotion gates for the production maintenance cycle

---

# 1. Purpose

This document defines the nightly retraining and refresh cycle for the system.

It answers five questions:

1. How many forecaster trainings should run nightly?
2. How many RL trainings should run nightly?
3. What is refreshed every night versus only during major campaigns?
4. What counts as a valid nightly candidate?
5. When is a nightly candidate allowed to replace the current promoted model?

This is not the bootstrap campaign spec.
This is the recurring maintenance spec.

---

# 2. Nightly training principles

1. Nightly retraining is a maintenance cycle, not a full research campaign.
2. Nightly retraining must be time-bounded and operationally predictable.
3. The currently promoted model remains the serving champion unless a nightly candidate passes replacement gates.
4. The forecaster must refresh before RL policy refresh begins.
5. Nightly retraining must use strictly causal data boundaries.
6. Nightly retraining must never silently replace a champion model without evaluation and recorded evidence.
7. Nightly retraining may be skipped safely if required data quality, volume, or infrastructure conditions are not met.

---

# 3. Nightly cycle overview

Each nightly run has five stages:

1. data freshness and integrity checks
2. forecaster candidate refresh
3. forecast packet regeneration for the latest window
4. RL policy candidate refresh
5. champion-versus-candidate evaluation and promotion decision

The nightly cycle produces **candidate models**, not automatic champions by default.

---

# 4. Canonical nightly defaults

## 4.1 Forecaster nightly defaults

Use:

- rolling train window: latest configured training window
- validation window: latest configured validation window immediately after train
- test / holdout comparison window: most recent completed evaluation slice not used for fitting
- seeds: `2`
- total forecaster runs per night: `2`
- max epochs per run: `20`
- early stopping patience: `5`

## 4.2 RL nightly defaults

Use:

- algorithm count: `1`
- canonical nightly algorithm: current promoted policy family
- seeds: `2`
- total RL runs per night: `2`
- environment steps per run:
  - minimum: `250,000`
  - preferred: `500,000`
  - maximum nightly budget: `1,000,000`

## 4.3 Total nightly run count

Default nightly training volume:

- forecaster runs: `2`
- RL runs: `2`

Total nightly jobs:

- `4`

This is the canonical default nightly cycle.

---

# 5. What nightly retraining is allowed to change

Nightly retraining may refresh:

1. forecaster weights
2. policy weights
3. rolling normalization state
4. regime estimator state
5. conformal calibration state
6. packet snapshots
7. evaluation metrics
8. candidate model registry entries

Nightly retraining must not silently change:

1. feature schema
2. tensor contracts
3. reward family
4. action semantics
5. simulator cost model
6. model architecture family
7. primary algorithm family

Those changes belong to a major campaign, not the nightly cycle.

---

# 6. Nightly ordering

The nightly cycle must run in this order:

## Stage 1 — data validation
1. verify the latest data snapshot exists
2. verify no catastrophic gaps beyond tolerated thresholds
3. verify feature schema version matches the serving system
4. verify cost-model inputs are available
5. verify there is enough newly completed data to justify retraining

## Stage 2 — forecaster nightly refresh
1. fit or update normalization state using the current nightly train window
2. fit or update regime estimator state using the same nightly train window
3. train nightly forecaster candidates across the configured seed list
4. evaluate candidates on validation and holdout slices
5. select the nightly forecaster candidate

## Stage 3 — forecast packet regeneration
1. generate causal forecast packets using the nightly forecaster candidate
2. persist packet snapshot and metadata
3. verify packet schema compatibility

## Stage 4 — RL nightly refresh
1. train nightly RL candidates using the regenerated packet snapshot
2. evaluate candidates on validation and holdout slices
3. select the nightly RL candidate

## Stage 5 — promotion decision
1. compare nightly candidates against the current champions
2. promote only if replacement gates are passed
3. otherwise keep the current champions in service
4. record the decision and rationale

---

# 7. Nightly forecaster spec

## 7.1 Number of runs

Default nightly forecaster campaign:

- seeds: `2`
- total nightly forecaster runs: `2`

Canonical seed list:

- `101`
- `202`

## 7.2 What counts as one nightly forecaster run

One nightly forecaster run means:

1. load the current nightly train/validation split definition
2. fit normalization state on train only
3. fit regime estimator state on train only
4. train the forecaster
5. validate during training
6. apply early stopping
7. freeze best candidate checkpoint
8. evaluate on the nightly holdout comparison slice
9. record metrics and artifacts

## 7.3 Epoch budget

Nightly forecaster budget:

- max epochs: `20`
- early stopping patience: `5`

## 7.4 Required nightly forecaster metrics

Each nightly forecaster candidate must record:

1. quantile loss
2. median MAE
3. directional accuracy
4. interval coverage
5. mean interval width
6. calibration error
7. regime-stratified summary
8. candidate-versus-champion comparison summary

## 7.5 Nightly forecaster candidate selection

Select the nightly forecaster candidate by aggregate result across the two nightly seeds.

Do not select based on a single best seed.

---

# 8. Nightly forecast packet generation

## 8.1 Purpose

The RL nightly refresh must consume forecast packets produced by the nightly forecaster candidate or the current champion forecaster, depending on the configured policy.

## 8.2 Default policy

Default nightly policy:

- use the selected nightly forecaster candidate to generate packet snapshots for the RL nightly refresh
- if nightly forecaster retraining fails, fall back to the current promoted forecaster for packet generation
- if both packet generation paths fail, skip RL nightly retraining

## 8.3 Packet requirements

Each nightly packet snapshot must include:

1. timestamp
2. horizons
3. lower quantiles
4. median quantiles
5. upper quantiles
6. interval widths
7. regime vector
8. confidence score
9. ensemble variance if available
10. diagnostics payload
11. packet schema version
12. source checkpoint identifier

---

# 9. Nightly RL policy spec

## 9.1 Number of runs

Default nightly RL campaign:

- algorithm family: current promoted policy family only
- seeds: `2`
- total nightly RL runs: `2`

Canonical seed list:

- `111`
- `222`

## 9.2 What counts as one nightly RL run

One nightly RL run means:

1. load nightly train, validation, and holdout segment definitions
2. load the nightly packet snapshot
3. initialize the simulator with frozen nightly cost assumptions
4. train the RL policy
5. checkpoint candidate policy
6. evaluate candidate on the nightly holdout slice
7. record metrics and artifacts

## 9.3 Environment-step budget

Nightly RL budget:

- minimum: `250,000`
- preferred: `500,000`
- maximum: `1,000,000`

## 9.4 Required nightly RL metrics

Each nightly RL candidate must record:

1. total return
2. Sharpe-like metric
3. Sortino-like metric
4. max drawdown
5. turnover
6. trade count
7. average holding time
8. average cost per trade
9. exposure utilization
10. abstention rate
11. regime-stratified summary
12. candidate-versus-champion comparison summary

## 9.5 Nightly RL candidate selection

Select the nightly RL candidate by aggregate result across the two nightly seeds.

Do not select based on a single best seed.

---

# 10. Champion-versus-candidate policy

## 10.1 Default serving rule

The current champion remains in service unless the nightly candidate passes replacement gates.

## 10.2 Replacement gates for the forecaster

A nightly forecaster candidate may replace the champion only if:

1. both nightly seed runs completed successfully
2. there is no schema mismatch
3. there is no evidence of leakage
4. aggregate validation/holdout performance is not worse than the champion beyond tolerated regression thresholds
5. calibration is acceptable
6. no operational artifact is missing

## 10.3 Replacement gates for the RL policy

A nightly RL candidate may replace the champion only if:

1. both nightly seed runs completed successfully
2. there is no simulator/config mismatch
3. aggregate holdout performance is not worse than the champion beyond tolerated regression thresholds
4. drawdown is not materially worse beyond the configured limit
5. turnover and costs remain within acceptable bounds
6. required artifacts and traces are present

---

# 11. Regression thresholds

## 11.1 Purpose

Nightly promotion requires explicit regression controls.

## 11.2 Forecaster regression rule

Do not replace the champion forecaster if the nightly candidate:

1. degrades primary quantile loss beyond configured tolerance
2. materially worsens calibration
3. materially widens intervals without compensating benefit
4. shows unstable variance across its seed pair

## 11.3 RL regression rule

Do not replace the champion policy if the nightly candidate:

1. materially worsens risk-adjusted return
2. materially worsens drawdown
3. materially increases turnover without sufficient benefit
4. shows unstable results across its seed pair

## 11.4 Threshold source

Exact numeric thresholds must be defined in config and must not be hard-coded in scattered locations.

---

# 12. Nightly fallback rules

If the nightly cycle fails at any stage:

1. keep the current champion models serving
2. record the failure reason
3. persist partial artifacts only if they are valid and clearly marked as incomplete
4. do not promote incomplete or partially evaluated candidates

Fallback cases:

## 12.1 Forecaster failure
- keep current champion forecaster
- optionally skip RL nightly refresh or run RL against the champion forecaster packets if configured

## 12.2 RL failure
- keep current champion RL policy
- nightly forecaster candidate may still be promoted independently if it passed its own gates

## 12.3 Data failure
- skip all nightly retraining
- keep current champions
- raise operational alert / run summary

---

# 13. Nightly skip rules

Nightly retraining may be skipped intentionally if:

1. insufficient new completed data is available
2. data quality checks fail
3. required services or artifacts are unavailable
4. the nightly time budget would be exceeded
5. the run is under manual freeze

A skipped run is valid if it is recorded cleanly and champions remain unchanged.

---

# 14. Candidate artifact requirements

## 14.1 Forecaster nightly artifacts

Each nightly forecaster candidate must emit:

1. checkpoint
2. config snapshot
3. normalization state
4. regime estimator state
5. validation metrics
6. holdout metrics
7. training log
8. schema version
9. data snapshot identifier

## 14.2 RL nightly artifacts

Each nightly RL candidate must emit:

1. policy checkpoint
2. value checkpoint if separate
3. config snapshot
4. reward config
5. simulator config
6. validation metrics
7. holdout metrics
8. turnover/cost summary
9. action trace summary
10. forecast packet snapshot identifier
11. data snapshot identifier

---

# 15. Nightly reporting requirements

Each nightly cycle must produce a final report containing:

1. data snapshot identifier
2. whether forecaster nightly retraining ran
3. whether RL nightly retraining ran
4. nightly candidate metrics
5. champion metrics used for comparison
6. promotion or non-promotion decision
7. reasons for each decision
8. any failures or skips
9. final serving model identifiers after the run

---

# 16. Coding-agent execution rules

A coding agent implementing the nightly cycle must do the following:

## 16.1 Before training
1. validate the latest data snapshot
2. validate schema compatibility
3. load nightly config
4. resolve champion model identifiers
5. enumerate the nightly seed list

## 16.2 Forecaster stage
1. run both nightly forecaster seed jobs
2. collect metrics in a common comparison table
3. choose the nightly forecaster candidate only after both jobs finish or fail conclusively

## 16.3 RL stage
1. generate or load valid packet snapshots
2. run both nightly RL seed jobs
3. compare nightly RL candidates under identical simulator and reward settings

## 16.4 Promotion stage
1. compare nightly candidates to champions
2. apply promotion gates
3. never silently replace a champion
4. record the final decision

## 16.5 Failure discipline
1. never treat missing artifacts as success
2. never promote partially validated candidates
3. never overwrite champion pointers before promotion gates are passed

---

# 17. Major-campaign trigger rules

Nightly retraining is not a substitute for a major campaign.

A major campaign should be triggered when any of the following occurs:

1. architecture family changes
2. feature schema changes
3. tensor widths change
4. reward family changes
5. simulator cost model changes
6. serving degradation persists across multiple nights
7. market regime shift is severe enough that nightly refresh no longer stabilizes performance

When a major campaign is triggered, use the bootstrap or full offline campaign spec instead of the nightly spec.

---

# 18. Canonical nightly summary

## 18.1 Default nightly cycle

### Forecaster
- seeds: `2`
- total runs: `2`
- max epochs: `20`
- patience: `5`

### RL
- algorithm family: current promoted family only
- seeds: `2`
- total runs: `2`
- env steps per run: `250k minimum`, `500k preferred`

### Total nightly jobs
- `4`

## 18.2 Nightly operating model

- produce candidates
- compare against champions
- promote only on passed gates
- otherwise keep current serving models unchanged

---

# 19. Final rule

Nightly retraining is a bounded maintenance routine.

Its job is to refresh candidates, evaluate them against the current champions, and only promote replacements when the evidence is good enough.
