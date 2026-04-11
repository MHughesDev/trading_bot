# Master System Pipeline Specification
## End-to-End Forecaster + Policy + Training + Nightly Refresh Architecture

Version: 1.0  
Document type: master implementation specification  
Scope: full system architecture, runtime pipeline, object model, training campaigns, nightly refresh, promotion logic, and artifact contracts  
Target use case: short-horizon crypto trading system with a probabilistic forecaster, reinforcement-learning policy, deterministic risk layer, and bounded maintenance cycle

---

# 1. Purpose

This document unifies the following previously separate specification areas into one master system specification:

1. the forecaster model,
2. the policy model,
3. the initial offline bootstrap training campaign,
4. the nightly training and refresh cycle.

The goal of this document is to define, in one place, exactly how the full system works, what each subsystem is responsible for, how data and decisions move through the pipeline, how training is performed, how candidate models are promoted, and what objects must exist in code.

This document is the authoritative system-level spec.

---

# 2. System Mission

The system exists to perform the following end-to-end function:

1. ingest market data,
2. convert raw market data into structured features,
3. produce calibrated multi-horizon forecasts of future returns and uncertainty,
4. convert those forecasts into target exposure decisions,
5. apply deterministic risk controls,
6. transform approved targets into executable trade plans,
7. update portfolio state,
8. retrain and refresh candidate models under controlled offline and nightly procedures.

The system is therefore divided into four major layers:

1. **Forecaster Layer**
2. **Policy Layer**
3. **Execution and Risk Layer**
4. **Training and Promotion Layer**

---

# 3. High-Level System Decomposition

The canonical full system is:

**Market Data → Feature Engineering → Normalization → Regime Estimation → Forecaster Model → Forecast Packet → Policy Observation Builder → Policy Model → Risk Gate → Execution Planner → Exchange Adapter → Portfolio Update**

The canonical training system is:

**Historical Data Snapshot → Walk-Forward Split Builder → Forecaster Training → Forecast Packet Generation → RL Environment Replay → Policy Training → Candidate Evaluation → Promotion Decision**

The canonical nightly system is:

**Nightly Data Validation → Nightly Forecaster Candidate Refresh → Packet Regeneration → Nightly Policy Candidate Refresh → Champion-versus-Candidate Evaluation → Promotion or No-Promotion Decision**

---

# 4. Non-Negotiable System Principles

The full system shall obey the following principles:

1. **Separation of concerns**  
   Forecasting, policy selection, risk control, execution, and orchestration remain separate.

2. **Probabilistic forecasting by default**  
   The forecaster publishes quantile-based forecasts, not point-only outputs.

3. **Position-based policy control**  
   The policy outputs target exposure, not exchange-order syntax.

4. **Deterministic risk boundary**  
   Risk constraints remain outside the policy and are enforced deterministically.

5. **Execution decoupling**  
   The execution layer translates approved target deltas into orders.

6. **Training / inference symmetry**  
   The live runtime and the training simulator must preserve the same state semantics.

7. **Time-respecting evaluation**  
   All training and validation must respect causal chronology.

8. **Candidate / champion discipline**  
   No model is promoted based on one lucky run.

9. **Schema stability**  
   Object contracts and tensor contracts are versioned and must not drift silently.

10. **No infrastructure assumptions**  
    This spec does not assume specific hardware, framework, cloud, orchestration vendor, or deployment substrate.

---

# 5. Canonical Runtime Pipeline

## 5.1 Runtime order

At each decision timestep, the system executes in the following order:

1. ingest latest market observation,
2. update rolling market features,
3. update normalization state,
4. update regime estimate,
5. construct forecaster input tensors,
6. run the forecaster,
7. produce a forecast packet,
8. run forecast guards,
9. read current portfolio state,
10. read current execution state,
11. read current risk state,
12. build the policy observation,
13. run the policy,
14. project the policy output into target position space,
15. evaluate the proposed target through the risk gate,
16. construct the execution plan,
17. submit orders only if economically meaningful,
18. update portfolio and realized state,
19. persist traces, diagnostics, and artifacts.

## 5.2 Runtime decision rule

The system reasons in two stages:

1. **What is likely to happen?** → forecaster
2. **Given what is likely to happen, what exposure should be held?** → policy

The policy does not decide how to place orders.
The policy decides what exposure should be held.
The execution layer decides how to express the approved target delta operationally.

---

# 6. Canonical Training Pipelines

## 6.1 Initial offline bootstrap campaign

The bootstrap campaign exists to identify stable promotable model families.

The canonical bootstrap order is:

1. freeze historical dataset snapshot,
2. freeze schema version and evaluation logic,
3. build walk-forward splits,
4. train all forecaster candidates,
5. evaluate and rank forecaster candidates,
6. select the promoted forecaster family,
7. generate causal forecast packets from the promoted forecaster,
8. train all policy candidates against those packets,
9. evaluate and rank policy candidates,
10. select the promoted policy family,
11. emit campaign reports and promotion outputs.

## 6.2 Nightly refresh cycle

The nightly cycle exists to refresh bounded candidate models and compare them to serving champions.

The canonical nightly order is:

1. validate latest completed data snapshot,
2. validate schema compatibility,
3. run nightly forecaster candidate refresh,
4. evaluate nightly forecaster candidate,
5. generate nightly forecast packet snapshot,
6. run nightly policy candidate refresh,
7. evaluate nightly policy candidate,
8. compare nightly candidates to current champions,
9. promote only if gates are passed,
10. otherwise keep current champions unchanged.

---

# 7. Global Canonical Defaults

These defaults are system-level reference values.

## 7.1 Forecaster defaults

- `history_length = 128`
- `forecast_horizon = 8`
- `regime_width = 4`
- `latent_width = 32`
- `recurrent_hidden_width = 128`
- `quantiles = [0.1, 0.5, 0.9]`
- `branch_scales = [1, 5, 20, 100]`
- `bootstrap_early_stopping_patience = 10`
- `nightly_early_stopping_patience = 5`

## 7.2 Policy defaults

- canonical action type = continuous target signed exposure
- action range = `[-1, 1]`
- policy training default = forecast-conditioned RL
- policy nightly algorithm family = current promoted family
- initial candidate algorithms = `PPO`, `SAC`

## 7.3 Bootstrap campaign defaults

- forecaster splits = `3`
- forecaster seeds per split = `3`
- total forecaster bootstrap runs = `9`
- RL splits = `3`
- RL seeds per algorithm per split = `3`
- RL algorithms = `2`
- total RL bootstrap runs = `18`

## 7.4 Nightly defaults

- nightly forecaster seeds = `2`
- nightly forecaster runs = `2`
- nightly RL seeds = `2`
- nightly RL runs = `2`
- nightly RL algorithm count = `1`
- nightly RL algorithm family = current promoted family
- nightly total jobs = `4`

---

# 8. Enumerated Master Object Model

This section defines the canonical system objects.

---

## Object 1 — `MarketObservation`

### Purpose
Represents one raw market event or bar used as the basis of all downstream processing.

### Required fields
1. `timestamp`
2. `open`
3. `high`
4. `low`
5. `close`
6. `volume`

### Optional fields
1. `bid_ask_spread`
2. `trade_count`
3. `trade_imbalance`
4. `depth_summary`
5. `mark_price`
6. `funding_rate`

### Example contract
```python
@dataclass
class MarketObservation:
    timestamp: Any
    open: float
    high: float
    low: float
    close: float
    volume: float
    bid_ask_spread: float | None = None
    trade_count: int | None = None
    trade_imbalance: float | None = None
    depth_summary: dict | None = None
    mark_price: float | None = None
    funding_rate: float | None = None
```

---

## Object 2 — `ObservedFeatureFrame`

### Purpose
Represents the engineered observed features derived from raw market observations.

### Required feature families
1. base-frequency return features,
2. volatility features,
3. volume features,
4. multi-window rolling summaries,
5. optional microstructure features,
6. optional regime-conditioned representations.

### Example contract
```python
@dataclass
class ObservedFeatureFrame:
    timestamp: Any
    feature_values: dict[str, float]
    schema_version: int
```

---

## Object 3 — `KnownFutureFeatureFrame`

### Purpose
Represents horizon-aligned future-known covariates used by the forecaster.

### Required feature families
1. minute-of-hour sine/cosine,
2. hour-of-day sine/cosine,
3. day-of-week sine/cosine,
4. optional session flags,
5. optional market-open / pre-close indicators.

### Example contract
```python
@dataclass
class KnownFutureFeatureFrame:
    timestamp: Any
    horizon_features: dict[int, dict[str, float]]
    schema_version: int
```

---

## Object 4 — `StaticFeatureFrame`

### Purpose
Represents invariant or slowly changing metadata supplied to the forecaster.

### Optional fields
1. `asset_id`
2. `venue_id`
3. `bar_interval_id`
4. `configuration_profile_id`

### Example contract
```python
@dataclass
class StaticFeatureFrame:
    values: dict[str, Any]
    schema_version: int
```

---

## Object 5 — `NormalizationState`

### Purpose
Represents rolling or regime-conditioned normalization state.

### Required fields
1. `global_stats`
2. `regime_stats`
3. `window_size`
4. `schema_version`

### Example contract
```python
@dataclass
class NormalizationState:
    global_stats: dict
    regime_stats: dict | None
    window_size: int
    schema_version: int
```

---

## Object 6 — `RegimeVector`

### Purpose
Represents the inferred soft market regime.

### Canonical regimes
1. uptrend
2. downtrend
3. low-volatility chop
4. high-volatility chop

### Example contract
```python
@dataclass
class RegimeVector:
    probabilities: list[float]  # length 4
    labels: list[str]
    timestamp: Any
```

---

## Object 7 — `ForecasterInputBatch`

### Purpose
Represents the full input batch to the forecaster model.

### Required tensors
1. `x_obs`
2. `x_known`
3. `x_static`
4. `regime_seq`
5. `regime_cur`

### Example contract
```python
@dataclass
class ForecasterInputBatch:
    x_obs: Any
    x_known: Any
    x_static: Any | None
    regime_seq: Any | None
    regime_cur: Any | None
    metadata: dict
```

---

## Object 8 — `ForecastPacket`

### Purpose
Represents the calibrated probabilistic belief emitted by the forecaster and consumed by the policy.

### Required fields
1. `timestamp`
2. `horizons`
3. `q_low`
4. `q_med`
5. `q_high`
6. `interval_width`
7. `regime_vector`
8. `confidence_score`
9. `ensemble_variance`
10. `ood_score`
11. `forecast_diagnostics`
12. `packet_schema_version`
13. `source_checkpoint_id`

### Example contract
```python
@dataclass
class ForecastPacket:
    timestamp: Any
    horizons: list[int]
    q_low: Any
    q_med: Any
    q_high: Any
    interval_width: Any
    regime_vector: Any
    confidence_score: Any | float
    ensemble_variance: Any
    ood_score: float
    forecast_diagnostics: dict
    packet_schema_version: int
    source_checkpoint_id: str | None = None
```

---

## Object 9 — `ForecastGuardResult`

### Purpose
Represents whether a forecast packet is acceptable for policy consumption.

### Required fields
1. `accepted`
2. `reasons`
3. `diagnostics`

### Example contract
```python
@dataclass
class ForecastGuardResult:
    accepted: bool
    reasons: list[str]
    diagnostics: dict
```

---

## Object 10 — `PortfolioState`

### Purpose
Represents the current holdings and capital state visible to the policy and execution stack.

### Required fields
1. `equity`
2. `cash`
3. `position_units`
4. `position_notional`
5. `position_fraction`
6. `entry_price`
7. `unrealized_pnl`
8. `realized_pnl`
9. `current_leverage`
10. `time_in_position`
11. `last_action`
12. `last_trade_timestamp`

### Example contract
```python
@dataclass
class PortfolioState:
    equity: float
    cash: float
    position_units: float
    position_notional: float
    position_fraction: float
    entry_price: float | None
    unrealized_pnl: float
    realized_pnl: float
    current_leverage: float
    time_in_position: int
    last_action: dict | None
    last_trade_timestamp: Any | None
```

---

## Object 11 — `ExecutionState`

### Purpose
Represents current execution-relevant microstructure conditions.

### Required fields
1. `mid_price`
2. `spread`
3. `estimated_slippage`
4. `estimated_fee_rate`
5. `available_liquidity_score`
6. `latency_proxy`
7. `volatility_proxy`

### Optional fields
1. `order_book_imbalance`
2. `recent_trade_flow`

### Example contract
```python
@dataclass
class ExecutionState:
    mid_price: float
    spread: float
    estimated_slippage: float
    estimated_fee_rate: float
    available_liquidity_score: float
    latency_proxy: float
    volatility_proxy: float
    order_book_imbalance: float | None = None
    recent_trade_flow: float | None = None
```

---

## Object 12 — `RiskState`

### Purpose
Represents active hard limits and risk envelopes.

### Required fields
1. `max_abs_position_fraction`
2. `max_position_delta_per_step`
3. `max_leverage`
4. `min_trade_notional`
5. `cooldown_steps_remaining`
6. `allow_long`
7. `allow_short`
8. `kill_switch_active`
9. `max_drawdown_limit`
10. `concentration_limit`
11. `volatility_limit`
12. `daily_loss_limit_remaining`

### Example contract
```python
@dataclass
class RiskState:
    max_abs_position_fraction: float
    max_position_delta_per_step: float
    max_leverage: float
    min_trade_notional: float
    cooldown_steps_remaining: int
    allow_long: bool
    allow_short: bool
    kill_switch_active: bool
    max_drawdown_limit: float
    concentration_limit: float
    volatility_limit: float
    daily_loss_limit_remaining: float
```

---

## Object 13 — `PolicyObservation`

### Purpose
Represents the structured state consumed by the policy network.

### Required groups
1. `forecast_features`
2. `portfolio_features`
3. `execution_features`
4. `risk_features`

### Optional groups
1. `history_features`
2. `metadata`

### Example contract
```python
@dataclass
class PolicyObservation:
    forecast_features: Any
    portfolio_features: Any
    execution_features: Any
    risk_features: Any
    history_features: Any | None
    metadata: dict
```

---

## Object 14 — `PolicyAction`

### Purpose
Represents the raw policy output before risk gating.

### Canonical v1 action
- `target_exposure ∈ [-1, 1]`

### Example contract
```python
@dataclass
class PolicyAction:
    target_exposure: float
    action_diagnostics: dict
```

---

## Object 15 — `TargetPosition`

### Purpose
Represents the projected desired position after interpreting the policy output.

### Required fields
1. `target_fraction`
2. `target_units`
3. `target_notional`
4. `target_leverage`
5. `reason_codes`

### Example contract
```python
@dataclass
class TargetPosition:
    target_fraction: float
    target_units: float | None
    target_notional: float | None
    target_leverage: float
    reason_codes: list[str]
```

---

## Object 16 — `ApprovedTarget`

### Purpose
Represents the risk-approved target.

### Required fields
1. `approved`
2. `approved_target_fraction`
3. `rejection_reasons`
4. `clamp_reasons`
5. `risk_diagnostics`

### Example contract
```python
@dataclass
class ApprovedTarget:
    approved: bool
    approved_target_fraction: float
    rejection_reasons: list[str]
    clamp_reasons: list[str]
    risk_diagnostics: dict
```

---

## Object 17 — `ExecutionPlan`

### Purpose
Represents the operational trade plan used to bridge approved target and exchange instructions.

### Required fields
1. `required_delta_fraction`
2. `required_delta_notional`
3. `execution_mode`
4. `max_child_order_size`
5. `urgency`
6. `skip_execution`
7. `skip_reasons`

### Example contract
```python
@dataclass
class ExecutionPlan:
    required_delta_fraction: float
    required_delta_notional: float
    execution_mode: str
    max_child_order_size: float | None
    urgency: float
    skip_execution: bool
    skip_reasons: list[str]
```

---

## Object 18 — `ExecutionResult`

### Purpose
Represents realized order-placement outcome and fill summary.

### Required fields
1. `submitted`
2. `accepted`
3. `orders`
4. `fills`
5. `fees_paid`
6. `slippage_realized`
7. `diagnostics`

### Example contract
```python
@dataclass
class ExecutionResult:
    submitted: bool
    accepted: bool
    orders: list[dict]
    fills: list[dict]
    fees_paid: float
    slippage_realized: float
    diagnostics: dict
```

---

## Object 19 — `TransitionRecord`

### Purpose
Represents a single RL training transition.

### Required fields
1. `observation_t`
2. `action_t`
3. `reward_t`
4. `observation_t1`
5. `done`
6. `info`

### Example contract
```python
@dataclass
class TransitionRecord:
    observation_t: PolicyObservation
    action_t: PolicyAction
    reward_t: float
    observation_t1: PolicyObservation
    done: bool
    info: dict
```

---

## Object 20 — `ForecasterCandidateArtifact`

### Purpose
Represents one forecaster candidate training result.

### Required fields
1. `checkpoint_path`
2. `config_snapshot`
3. `normalization_state_path`
4. `regime_estimator_state_path`
5. `validation_metrics`
6. `test_or_holdout_metrics`
7. `training_log_path`
8. `schema_version`
9. `data_snapshot_id`

### Example contract
```python
@dataclass
class ForecasterCandidateArtifact:
    checkpoint_path: str
    config_snapshot: dict
    normalization_state_path: str
    regime_estimator_state_path: str
    validation_metrics: dict
    test_or_holdout_metrics: dict
    training_log_path: str
    schema_version: int
    data_snapshot_id: str
```

---

## Object 21 — `PolicyCandidateArtifact`

### Purpose
Represents one policy candidate training result.

### Required fields
1. `policy_checkpoint_path`
2. `value_checkpoint_path`
3. `config_snapshot`
4. `reward_config`
5. `simulator_config`
6. `validation_metrics`
7. `test_or_holdout_metrics`
8. `action_trace_summary`
9. `turnover_cost_summary`
10. `data_snapshot_id`
11. `forecast_packet_snapshot_id`

### Example contract
```python
@dataclass
class PolicyCandidateArtifact:
    policy_checkpoint_path: str
    value_checkpoint_path: str | None
    config_snapshot: dict
    reward_config: dict
    simulator_config: dict
    validation_metrics: dict
    test_or_holdout_metrics: dict
    action_trace_summary: dict
    turnover_cost_summary: dict
    data_snapshot_id: str
    forecast_packet_snapshot_id: str
```

---

## Object 22 — `ForecastPacketSnapshot`

### Purpose
Represents a persisted packet dataset used for policy training or nightly refresh.

### Required fields
1. `snapshot_id`
2. `source_forecaster_checkpoint`
3. `packet_schema_version`
4. `data_snapshot_id`
5. `time_range`
6. `packet_count`
7. `storage_path`

### Example contract
```python
@dataclass
class ForecastPacketSnapshot:
    snapshot_id: str
    source_forecaster_checkpoint: str
    packet_schema_version: int
    data_snapshot_id: str
    time_range: dict
    packet_count: int
    storage_path: str
```

---

## Object 23 — `PromotionDecision`

### Purpose
Represents a champion-versus-candidate decision record.

### Required fields
1. `component`
2. `current_champion_id`
3. `candidate_id`
4. `decision`
5. `reasons`
6. `comparison_metrics`
7. `timestamp`

### Example contract
```python
@dataclass
class PromotionDecision:
    component: str
    current_champion_id: str | None
    candidate_id: str | None
    decision: str
    reasons: list[str]
    comparison_metrics: dict
    timestamp: Any
```

---

## Object 24 — `SystemRunReport`

### Purpose
Represents the final structured report for a bootstrap or nightly cycle.

### Required fields
1. `run_type`
2. `data_snapshot_id`
3. `forecaster_summary`
4. `policy_summary`
5. `promotion_decisions`
6. `failures`
7. `final_serving_model_ids`

### Example contract
```python
@dataclass
class SystemRunReport:
    run_type: str
    data_snapshot_id: str
    forecaster_summary: dict
    policy_summary: dict
    promotion_decisions: list[PromotionDecision]
    failures: list[str]
    final_serving_model_ids: dict
```

---

# 9. Forecaster Subsystem Specification

## 9.1 Responsibility

The forecaster model exists to transform recent market state into a calibrated multi-horizon probabilistic belief about future returns.

## 9.2 Canonical forecaster structure

The canonical forecaster pipeline is:

**Observed Features → Variable Selection Network → Latent CNN Encoder → Multi-Resolution xLSTM Branches → Regime-Conditioned Fusion → Quantile Decoder → Conformal Calibration → Ensemble Aggregation**

## 9.3 Forecaster input semantics

The forecaster consumes:

1. observed historical tensor `X_obs`,
2. known future tensor `X_known`,
3. static tensor `X_static`,
4. regime sequence tensor `R_seq`,
5. current regime tensor `R_cur`.

## 9.4 Forecaster output semantics

The forecaster emits a `ForecastPacket`.

Required probabilistic outputs:

1. lower quantiles,
2. median quantiles,
3. upper quantiles,
4. interval widths,
5. regime vector,
6. confidence score,
7. ensemble disagreement,
8. forecast diagnostics.

## 9.5 Forecaster evaluation

Required forecaster metrics include:

1. quantile loss,
2. median MAE,
3. directional accuracy by horizon,
4. interval coverage probability,
5. mean interval width,
6. calibration error,
7. regime-stratified metrics,
8. time-of-day stratified metrics.

## 9.6 Forecaster guard conditions

The forecaster must support rejection checks for:

1. interval width above threshold,
2. ensemble disagreement above threshold,
3. out-of-distribution feature score above threshold,
4. missing feature values,
5. stale normalization state,
6. insufficient calibration history.

---

# 10. Policy Subsystem Specification

## 10.1 Responsibility

The policy model exists to convert the probabilistic belief from the forecaster into a target exposure decision.

## 10.2 Canonical policy structure

The canonical policy pipeline is:

**Forecast Packet + Portfolio State + Execution State + Risk State → Policy Observation → Policy Network → Policy Action → Action Projector → Target Position**

## 10.3 Canonical policy action

The first production-style policy action is:

`target_exposure ∈ [-1, 1]`

Interpretation:

- `-1` = maximum permitted short,
- `0` = flat,
- `+1` = maximum permitted long.

## 10.4 Policy observation groups

The policy observation must include:

1. forecast-derived features,
2. portfolio-derived features,
3. execution-derived features,
4. risk-derived features,
5. optional historical context features.

## 10.5 Policy reward family

The canonical reward family is:

`net_pnl - turnover_penalty - cost_penalty - drawdown_penalty - inventory_penalty - churn_penalty`

Optional shaped reward terms may include:

1. abstention reward under justified uncertainty,
2. reward for maintaining profitable exposure,
3. penalty for acting against forecast-confidence filters,
4. penalty for acting under OOD conditions.

## 10.6 Policy evaluation

Required policy metrics include:

1. total return,
2. Sharpe-like metric,
3. Sortino-like metric,
4. max drawdown,
5. turnover,
6. trade count,
7. average holding time,
8. average cost per trade,
9. exposure utilization,
10. abstention rate,
11. regime-stratified performance,
12. cost-aware performance.

---

# 11. Risk Subsystem Specification

## 11.1 Responsibility

The risk subsystem is a deterministic boundary that constrains what the policy is allowed to request.

## 11.2 Canonical risk checks

The risk gate must support:

1. kill-switch checks,
2. forecast confidence threshold checks,
3. interval width threshold checks,
4. ensemble disagreement threshold checks,
5. OOD checks,
6. max absolute exposure checks,
7. max per-step exposure change checks,
8. leverage checks,
9. cooldown checks,
10. drawdown checks,
11. daily loss limit checks,
12. volatility regime restriction checks,
13. minimum trade size checks.

## 11.3 Canonical risk outcomes

The risk gate may:

1. approve unchanged,
2. approve with clamp,
3. reject entirely,
4. convert to flat target.

---

# 12. Execution Subsystem Specification

## 12.1 Responsibility

The execution subsystem translates the approved target into economically meaningful order activity.

## 12.2 Canonical execution delta

Let:

- `current_fraction = portfolio_state.position_fraction`
- `approved_fraction = approved_target.approved_target_fraction`

Then:

`required_delta_fraction = approved_fraction - current_fraction`

If:

`abs(required_delta_fraction) < min_effective_change`

then skip execution.

## 12.3 Planner responsibilities

The execution planner must:

1. compute target delta,
2. evaluate economic significance,
3. choose execution mode,
4. assign urgency,
5. split the plan into child intents if needed.

---

# 13. Bootstrap Training Campaign Specification

## 13.1 Purpose

The bootstrap campaign is the initial full offline training program used to identify stable promotable model families.

## 13.2 Canonical bootstrap counts

### Forecaster
- splits: `3`
- seeds per split: `3`
- total runs: `9`

### RL
- algorithms: `PPO`, `SAC`
- splits: `3`
- seeds per algorithm per split: `3`
- total runs: `18`

## 13.3 Bootstrap ordering

1. freeze data snapshot,
2. freeze schema version and evaluation logic,
3. build walk-forward splits,
4. train all forecaster runs,
5. evaluate and rank forecaster candidates,
6. select promoted forecaster family,
7. generate causal packet snapshots,
8. train all RL runs,
9. evaluate and rank policy candidates,
10. select promoted policy family,
11. emit reports and promotion outputs.

## 13.4 Bootstrap minimum constrained mode

### Forecaster minimum
- splits: `2`
- seeds per split: `2`
- total runs: `4`

### RL minimum
- algorithm: `PPO`
- splits: `2`
- seeds per split: `3`
- total runs: `6`

---

# 14. Nightly Refresh Specification

## 14.1 Purpose

The nightly cycle is a bounded maintenance routine used to refresh candidate models and compare them against serving champions.

## 14.2 Canonical nightly counts

### Forecaster nightly
- seeds: `2`
- total runs: `2`
- max epochs: `20`
- early stopping patience: `5`

### RL nightly
- algorithm family: current promoted family only
- seeds: `2`
- total runs: `2`
- env steps per run:
  - minimum: `250,000`
  - preferred: `500,000`
  - maximum: `1,000,000`

### Total nightly jobs
- `4`

## 14.3 Nightly ordering

1. validate latest completed data snapshot,
2. validate schema compatibility,
3. train nightly forecaster candidates,
4. evaluate nightly forecaster candidates,
5. generate nightly packet snapshot,
6. train nightly policy candidates,
7. evaluate nightly policy candidates,
8. compare to current champions,
9. promote only if gates pass,
10. otherwise keep current champions unchanged.

## 14.4 Nightly candidate policy

The nightly cycle produces candidates, not automatic replacements.

The champion remains in service unless the nightly candidate passes replacement gates.

## 14.5 Nightly failure and fallback rules

If the nightly cycle fails at any stage:

1. keep current champion models serving,
2. record the failure reason,
3. do not promote incomplete candidates.

If nightly forecaster retraining fails:
- keep current champion forecaster,
- optionally use the champion forecaster to produce packets if configured.

If nightly policy retraining fails:
- keep current champion policy.

If data validation fails:
- skip nightly retraining,
- keep current champions,
- emit run summary and alert condition.

---

# 15. Candidate and Champion Promotion Logic

## 15.1 Bootstrap promotion logic

Bootstrap promotion is based on aggregate evidence across all required runs.

No component may be promoted based on a single exceptional run.

## 15.2 Nightly promotion logic

Nightly promotion is based on candidate-versus-champion comparison under fixed schema and fixed reward/cost assumptions.

## 15.3 Forecaster promotion gates

A forecaster candidate may be promoted only if:

1. all required runs complete,
2. there is no leakage,
3. there is no schema mismatch,
4. calibration is acceptable,
5. aggregate performance is stable across seeds,
6. no competing candidate is clearly better on the aggregate objective.

## 15.4 Policy promotion gates

A policy candidate may be promoted only if:

1. all required runs complete,
2. results are not dominated by one lucky seed,
3. cost-aware performance remains acceptable,
4. drawdown is not structurally unacceptable,
5. the candidate beats the alternative on aggregate or is materially simpler with comparable performance.

---

# 16. Canonical Seed Plan

## 16.1 Bootstrap seeds

### Forecaster bootstrap seeds
- `101`
- `202`
- `303`

### RL bootstrap seeds
- `111`
- `222`
- `333`

## 16.2 Nightly seeds

### Forecaster nightly seeds
- `101`
- `202`

### RL nightly seeds
- `111`
- `222`

These seed lists must be frozen before the run begins.

---

# 17. Artifact and Serialization Scope

## 17.1 Forecaster artifacts

Each forecaster run must emit:

1. checkpoint,
2. config snapshot,
3. normalization state,
4. regime estimator state,
5. validation metrics,
6. test or holdout metrics,
7. training log,
8. schema version,
9. data snapshot identifier.

## 17.2 Policy artifacts

Each policy run must emit:

1. policy checkpoint,
2. value checkpoint if separate,
3. config snapshot,
4. reward config,
5. simulator config,
6. validation metrics,
7. test or holdout metrics,
8. action trace summary,
9. turnover and cost summary,
10. data snapshot identifier,
11. forecast packet snapshot identifier.

## 17.3 System serialization rule

The system must serialize independently:

1. model weights,
2. optimizer state,
3. scheduler state,
4. normalization state,
5. regime estimator state,
6. conformal calibrator state,
7. feature schema version,
8. packet schema version,
9. training metadata,
10. promotion decision records.

---

# 18. End-to-End Interface Summary

## 18.1 Forecaster interface

```python
class ForecasterModel:
    def forward(self, x_obs, x_known, x_static=None, regime_seq=None, regime_cur=None) -> dict:
        ...
```

## 18.2 Forecast guard interface

```python
class ForecastGuard:
    def check(self, forecast_packet, metadata) -> ForecastGuardResult:
        ...
```

## 18.3 Policy observation builder interface

```python
class PolicyObservationBuilder:
    def build(self, forecast_packet, portfolio_state, execution_state, risk_state, history_context=None) -> PolicyObservation:
        ...
```

## 18.4 Policy algorithm interface

```python
class PolicyAlgorithm:
    def select_action(self, obs: PolicyObservation, deterministic: bool = True) -> PolicyAction:
        ...
    def update(self, batch) -> dict:
        ...
```

## 18.5 Action projector interface

```python
class ActionProjector:
    def project(self, action: PolicyAction, portfolio_state: PortfolioState, risk_state: RiskState) -> TargetPosition:
        ...
```

## 18.6 Risk gate interface

```python
class RiskGate:
    def evaluate(self, target: TargetPosition, forecast_packet: ForecastPacket, portfolio_state: PortfolioState, execution_state: ExecutionState, risk_state: RiskState) -> ApprovedTarget:
        ...
```

## 18.7 Execution planner interface

```python
class ExecutionPlanner:
    def plan(self, approved_target: ApprovedTarget, portfolio_state: PortfolioState, execution_state: ExecutionState) -> ExecutionPlan:
        ...
```

## 18.8 RL environment interface

```python
class TradingPolicyEnvironment:
    def reset(self) -> PolicyObservation:
        ...
    def step(self, action: PolicyAction):
        ...
```

---

# 19. Canonical Package-Level Partitioning

A framework-neutral logical partitioning is:

```text
master_system/
  config/
  data/
  features/
  normalization/
  regime/
  forecaster/
  forecasting_packets/
  policy/
  risk/
  execution/
  portfolio/
  training/
    bootstrap/
    nightly/
  evaluation/
  artifacts/
  promotion/
  inference/
  utils/
```

This package layout is conceptual and not mandatory in syntax, but the separation of responsibilities is mandatory.

---

# 20. Final Canonical Definition

The canonical system defined by this document is:

**an end-to-end, forecast-conditioned trading architecture in which raw market observations are transformed into normalized and regime-aware feature tensors, processed by a probabilistic multi-horizon forecaster, emitted as calibrated forecast packets, consumed by a reinforcement-learning policy that outputs target exposure, bounded by deterministic risk controls, translated into execution plans, evaluated under walk-forward campaigns, and refreshed nightly under candidate-versus-champion promotion rules.**

In short form:

**Market Data → Features → Forecaster → Forecast Packet → Policy → Risk Gate → Execution Plan → Portfolio Update → Bootstrap / Nightly Retraining → Promotion**

---

# 21. Most Important System Rule

The most important architectural rule is:

**The forecaster predicts, the policy decides exposure, the risk layer constrains, and the execution layer operationalizes.**

No single subsystem is allowed to collapse those responsibilities together.
