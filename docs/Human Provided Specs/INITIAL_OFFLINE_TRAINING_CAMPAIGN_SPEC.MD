# Initial Offline Training Campaign Spec
## Forecaster + RL Policy Bootstrap

Version: 1.0  
Document type: execution spec  
Scope: one-time initial offline training campaign for the forecaster and policy model  
Purpose: define the exact number of runs, splits, seeds, validation rules, and promotion gates for the first serious training campaign before any nightly retraining exists

---

# 1. Purpose

This document defines the initial training campaign for the system.

It answers four questions:

1. How many forecaster trainings should be run initially?
2. How many RL trainings should be run initially?
3. What counts as one completed run?
4. What promotion criteria must be satisfied before a model is considered usable?

This is not the nightly retrain spec.
This is the initial offline bootstrap campaign.

---

# 2. Non-negotiable training principles

1. No single-run conclusions are allowed.
2. All training and evaluation must use time-respecting walk-forward splits.
3. The forecaster must be trained and frozen before policy training consumes its outputs.
4. The RL policy must be trained against a cost-aware simulator.
5. Promotion decisions must be based on aggregate results across seeds and splits, not a single best run.
6. Validation and test windows must remain strictly unseen at the time of fitting.

---

# 3. Fixed campaign defaults

## 3.1 Forecaster-aligned defaults

Use the following canonical values unless the config explicitly overrides them:

- history length: `L = 128`
- forecast horizon: `H = 8`
- regime width: `F_regime = 4`
- latent width: `D_latent = 32`
- recurrent hidden width: `D_hidden = 128`
- quantiles: `[0.1, 0.5, 0.9]`
- early stopping patience: `10`
- walk-forward evaluation: required

## 3.2 Initial campaign scale

### Forecaster campaign
- walk-forward splits: `3`
- random seeds per split: `3`
- total forecaster runs: `9`

### RL campaign
- candidate algorithms: `2`
- required initial algorithms: `PPO` and `SAC`
- walk-forward splits: `3`
- random seeds per algorithm: `3`
- total RL runs: `18`

Formula:
- forecaster total = `3 splits × 3 seeds = 9`
- RL total = `2 algorithms × 3 splits × 3 seeds = 18`

## 3.3 Minimum reduced campaign when constrained

If compute or time is limited, the minimum serious campaign is:

### Forecaster minimum
- walk-forward splits: `2`
- random seeds per split: `2`
- total forecaster runs: `4`

### RL minimum
- algorithms: `1`
- required initial algorithm if only one is used: `PPO`
- walk-forward splits: `2`
- random seeds per split: `3`
- total RL runs: `6`

This reduced mode is allowed only when explicitly requested.

---

# 4. Campaign ordering

The campaign must run in the following order:

## Stage 1 — data freeze
1. freeze the historical dataset snapshot used for the campaign
2. freeze feature schema version
3. freeze cost-model assumptions
4. freeze evaluation code version

## Stage 2 — forecaster campaign
1. build walk-forward splits
2. train the forecaster across all split/seed combinations
3. evaluate each trained forecaster on its corresponding unseen test segment
4. rank forecaster candidates by aggregate performance
5. select the promoted forecaster family for packet generation

## Stage 3 — forecast packet generation
1. run the selected forecaster causally across each policy training segment
2. persist forecast packets for RL training
3. persist packet metadata, including confidence, interval width, regime vector, and ensemble variance if available

## Stage 4 — RL campaign
1. train each candidate RL algorithm against the simulator using the persisted forecast packets
2. evaluate each trained policy on unseen test segments
3. compare aggregate outcomes across algorithms, seeds, and splits
4. select the promoted policy family

## Stage 5 — confirmation pass
1. re-run the winning forecaster configuration for one additional confirmation seed set if results were borderline
2. re-run the winning RL algorithm for one additional confirmation seed set if results were borderline

---

# 5. Walk-forward split design

## 5.1 General rule

Every split must contain:

1. train segment
2. validation segment
3. test segment

The order must always be:

`train -> validation -> test`

No shuffling is allowed.

## 5.2 Canonical count

Use `3` walk-forward splits for the default campaign.

## 5.3 Example structure

For a dataset ordered by time:

- split 1:
  - train: earliest segment
  - validation: next segment
  - test: next segment

- split 2:
  - train: extended forward window
  - validation: next segment
  - test: next segment

- split 3:
  - train: later forward window
  - validation: next segment
  - test: next segment

Exact date boundaries are dataset-dependent and must be computed from the available history.

## 5.4 Split quality requirement

Each split should contain more than one market regime whenever possible.

A split is weak if it contains only one calm, homogeneous regime.

---

# 6. Forecaster training spec

## 6.1 Number of runs

The initial default forecaster campaign is:

- `3` splits
- `3` seeds per split
- total: `9` runs

## 6.2 What counts as one forecaster run

One forecaster run means:

1. fit normalization state on train only
2. fit regime estimator on train only
3. train model on train set
4. validate during training on validation set
5. apply early stopping
6. freeze best checkpoint
7. evaluate on test set
8. record metrics and artifacts

## 6.3 Epoch budget

Use:

- max epochs: `50`
- early stopping patience: `10`

Training may stop earlier if validation no longer improves.

## 6.4 Required recorded metrics

Each forecaster run must record at minimum:

1. quantile loss
2. median MAE
3. directional accuracy by horizon
4. interval coverage probability
5. mean interval width
6. calibration error
7. regime-stratified metrics
8. test-segment summary

## 6.5 Forecaster selection rule

Select the forecaster configuration based on aggregate performance across all splits and seeds.

Do not select by single best run.

Preferred ranking order:

1. primary: validation/test quantile loss stability
2. secondary: calibration quality
3. tertiary: directional consistency
4. quaternary: proxy cost-adjusted signal quality

## 6.6 Forecaster promotion gate

A forecaster candidate may be promoted only if:

1. it completes all assigned runs
2. no run shows obvious leakage or schema mismatch
3. aggregate performance is stable across seeds
4. calibration is not catastrophically broken
5. no competing configuration is clearly better on the aggregate objective

---

# 7. Forecast packet generation spec

## 7.1 Purpose

The RL stage must train on causal forecast packets, not on hindsight labels directly injected into the observation.

## 7.2 Required packet fields

Each persisted packet must include at minimum:

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

## 7.3 Generation rules

1. packets must be generated causally
2. no future leakage is allowed
3. packets must be versioned
4. packets must be reproducible from the promoted forecaster checkpoint and data snapshot

---

# 8. RL policy training spec

## 8.1 Number of runs

The default RL campaign is:

- algorithms: `PPO`, `SAC`
- splits: `3`
- seeds per algorithm per split: `3`

Total RL runs:

`2 × 3 × 3 = 18`

## 8.2 What counts as one RL run

One RL run means:

1. initialize the environment for one walk-forward split
2. load causal forecast packets for that split
3. train the policy on the train segment
4. tune and checkpoint on the validation segment according to the chosen algorithm logic
5. freeze the selected checkpoint
6. evaluate on the unseen test segment
7. record metrics, actions, turnover, and diagnostics

## 8.3 Environment-step budget

Initial default step budget per RL run:

- minimum: `1,000,000` environment steps
- preferred: `2,000,000`
- hard stop for initial bootstrap unless explicitly changed: `3,000,000`

## 8.4 Reward baseline

The default reward family is:

`net_pnl - turnover_penalty - cost_penalty - drawdown_penalty - churn_penalty`

The reward config must be frozen for the full campaign before comparison begins.

## 8.5 Required RL metrics

Each RL run must record at minimum:

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
11. regime-stratified performance
12. performance under cost-aware accounting

## 8.6 RL selection rule

Select the RL algorithm family by aggregate performance across all splits and seeds.

Do not promote a policy based on a single exceptional equity curve.

Preferred ranking order:

1. primary: risk-adjusted return stability
2. secondary: max drawdown control
3. tertiary: turnover discipline under costs
4. quaternary: regime consistency

## 8.7 RL promotion gate

A policy candidate may be promoted only if:

1. it completes all required runs
2. results are not dominated by one lucky seed
3. cost-aware performance remains acceptable
4. drawdown is not structurally unacceptable
5. it beats the alternative candidate on aggregate or is materially simpler with comparable performance

---

# 9. Canonical seed plan

## 9.1 Forecaster seeds

Use the same canonical seed list for each split:

- `101`
- `202`
- `303`

## 9.2 RL seeds

Use the same canonical seed list for each algorithm and split:

- `111`
- `222`
- `333`

The exact integers may be changed, but the list must be frozen before the campaign begins.

---

# 10. Artifact requirements

## 10.1 Forecaster artifacts

Each forecaster run must emit:

1. trained checkpoint
2. config snapshot
3. normalization state
4. regime estimator state
5. validation metrics
6. test metrics
7. training log
8. schema version
9. data snapshot identifier

## 10.2 RL artifacts

Each RL run must emit:

1. policy checkpoint
2. value checkpoint if separate
3. config snapshot
4. reward config
5. simulator config
6. validation metrics
7. test metrics
8. action trace summary
9. turnover/cost summary
10. data snapshot identifier
11. forecast packet snapshot identifier

---

# 11. Promotion outputs

At the end of the campaign, produce exactly these outputs:

## 11.1 Forecaster promotion output
1. promoted forecaster config
2. promoted checkpoint path
3. forecaster campaign report
4. rationale for promotion

## 11.2 Policy promotion output
1. promoted RL algorithm family
2. promoted policy config
3. promoted checkpoint path
4. policy campaign report
5. rationale for promotion

---

# 12. Failure rules

The campaign must fail and require review if any of the following occur:

1. evidence of temporal leakage
2. packet schema mismatch across stages
3. missing validation/test outputs
4. unstable or corrupted metrics logging
5. simulator costs omitted from RL evaluation
6. promoted model chosen from one best run instead of aggregate evidence

---

# 13. Coding-agent execution rules

A coding agent implementing this campaign must do the following:

## 13.1 Planning
1. enumerate all splits before training starts
2. enumerate all seeds before training starts
3. enumerate all algorithm/config combinations before training starts

## 13.2 Forecaster stage
1. run all forecaster jobs first
2. collect metrics in a comparable tabular format
3. choose the promoted forecaster only after all jobs finish or are explicitly marked failed

## 13.3 RL stage
1. generate or load causal forecast packets from the promoted forecaster
2. run all RL jobs against the same split definitions
3. compare candidate algorithms only under identical simulator and reward assumptions

## 13.4 Reporting
1. produce one aggregate summary table for forecaster runs
2. produce one aggregate summary table for RL runs
3. produce one promotion decision record for each stage
4. never silently skip failed jobs

---

# 14. Canonical initial campaign summary

## 14.1 Default serious campaign

### Forecaster
- splits: `3`
- seeds: `3`
- total runs: `9`
- max epochs: `50`
- patience: `10`

### RL
- algorithms: `PPO`, `SAC`
- splits: `3`
- seeds: `3`
- total runs: `18`
- env steps per run: `1M minimum`, `2M preferred`

## 14.2 Minimum acceptable campaign

### Forecaster
- splits: `2`
- seeds: `2`
- total runs: `4`

### RL
- algorithms: `PPO` only
- splits: `2`
- seeds: `3`
- total runs: `6`

---

# 15. Final rule

The initial offline training campaign is not a single fit.

It is a controlled training program whose objective is to identify a stable, promotable forecaster and a stable, promotable policy under walk-forward evaluation, multiple seeds, and cost-aware validation.
