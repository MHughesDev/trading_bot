# Code-Level Architecture Specification — Policy Model
## Forecast-Conditioned Reinforcement Learning Policy for Crypto Trading

Version: 1.0  
Document type: implementation specification  
Scope: policy model, data contracts, training design, inference pipeline, object model, interfaces, reward structure, risk gating, and system composition  
Target use case: short-horizon crypto trading where a probabilistic forecasting module already exists and produces calibrated forward beliefs

---

# 1. Purpose

This document defines the **policy model** that consumes the output of the probabilistic forecaster and converts that forecast into a **position decision**.

The policy model is **not** the forecaster.  
The policy model is **not** the execution engine.  
The policy model is **not** the exchange adapter.  
The policy model is **not** the historical feature builder.

The policy model is the layer that answers one question:

**Given the current portfolio state, the current market state, and the current forecast packet, what target exposure should the system hold right now?**

This architecture therefore formalizes a separation of concerns:

1. **World model / forecaster** produces beliefs about the future.
2. **Policy model** chooses target exposure given those beliefs.
3. **Risk layer** constrains what is allowed.
4. **Execution layer** converts the allowed target change into actual orders.
5. **Portfolio layer** records holdings, realized PnL, fees, and position state.

The central design principle is:

**The policy reasons in terms of target position, not raw order impulses.**

That means the policy should not directly output “market buy now” or “market sell now.”  
Instead, it should output something like:

- target inventory fraction,
- target signed exposure,
- target leverage,
- or target position delta.

This creates a stable interface between decision-making and execution.

---

# 2. Canonical System Placement

The full system is:

**Market Data → Feature Layer → Probabilistic Forecaster / World Model → Policy Model → Risk Gate → Execution Planner → Exchange Adapter → Portfolio Update**

The forecaster publishes a forecast packet.  
The policy consumes the forecast packet and returns a target decision object.  
The risk gate validates, clamps, rejects, or modifies that target.  
The execution planner transforms the approved target difference into executable child intents.

---

# 3. Policy Model Design Principles

## 3.1 Primary principles

The policy model shall satisfy the following principles:

1. **Forecast-conditioned decision-making**  
   The policy must explicitly consume output from the probabilistic world model.

2. **Position-based control**  
   The output must describe desired exposure, not direct exchange order syntax.

3. **Continuous-action friendliness**  
   The policy must support continuous target sizing.

4. **Risk-aware reward structure**  
   The objective must optimize risk-adjusted economic behavior, not naive next-step profit only.

5. **Non-myopic control**  
   The policy must reason over future consequences, not just immediate return sign.

6. **Execution decoupling**  
   The policy must not assume a specific order type, venue API, or matching engine behavior.

7. **Guard-compatible operation**  
   The policy must support rejection when forecast confidence is insufficient.

8. **Training / inference symmetry**  
   The same state semantics must exist in both backtest and live inference.

9. **Environment realism**  
   Costs, spread, slippage, and inventory persistence must be represented during training.

10. **No infrastructure assumptions**  
    The spec does not assume a specific framework, hardware target, deployment runtime, or cloud substrate.

## 3.2 Modeling principles

The following modeling choices are canonical for the first implementation:

1. the world model remains external to the policy,
2. the policy receives world-model outputs as structured state,
3. the action is continuous target exposure,
4. the reward is cost-adjusted and risk-penalized,
5. risk constraints remain a separate deterministic layer,
6. execution remains deterministic or rule-based relative to the approved target.

---

# 4. Functional Requirements

The policy system must support:

1. ingestion of forecast packets from the world model,
2. ingestion of portfolio state,
3. ingestion of market microstructure state where available,
4. ingestion of current risk limits,
5. state assembly into a policy observation,
6. action computation,
7. action projection into valid bounds,
8. risk gating,
9. target position generation,
10. optional rejection / abstention,
11. reward computation during training,
12. transition storage for RL training,
13. walk-forward retraining,
14. policy evaluation under realistic costs,
15. policy serialization,
16. online inference with deterministic guards.

---

# 5. Canonical Problem Definition

At each decision time `t`, define:

- `W_t` = forecast packet from the world model,
- `P_t` = portfolio state,
- `M_t` = current market / execution state,
- `R_t` = active risk state,
- `S_t` = policy observation assembled from these objects.

The policy computes:

`π_θ(S_t) -> A_t`

where `A_t` is the action.

The action is then mapped into a target position object:

`A_t -> T_t`

The risk layer transforms the proposal into an approved target:

`G(T_t, R_t, M_t, W_t) -> T'_t`

The execution layer converts the difference between the approved target and the current position into orders:

`E(T'_t - current_position_t) -> execution plan`

The environment evolves to the next state and yields reward:

`(S_t, A_t) -> S_(t+1), reward_t`

---

# 6. Policy Objective

The policy objective is to maximize long-run portfolio utility rather than raw directional accuracy.

A canonical one-step reward is:

`r_t = Δlog_equity_t - λ_turn * turnover_t - λ_cost * cost_t - λ_dd * drawdown_penalty_t - λ_inv * inventory_penalty_t - λ_churn * action_churn_t`

Where:

1. `Δlog_equity_t` = change in log equity over the step,
2. `turnover_t` = absolute traded notional or position change,
3. `cost_t` = spread + fees + slippage estimate,
4. `drawdown_penalty_t` = penalty for entering or deepening drawdown,
5. `inventory_penalty_t` = penalty for undesired or overlarge exposure,
6. `action_churn_t` = penalty for unnecessary oscillation.

The reward definition must remain configurable.

---

# 7. Canonical Policy Pipeline

## 7.1 End-to-end online pipeline

At each timestep:

1. market state is updated,
2. features are updated,
3. world model inference is run,
4. a forecast packet is published,
5. portfolio state is read,
6. risk state is read,
7. policy observation is assembled,
8. policy computes action,
9. action is transformed into target position,
10. risk gate clamps or rejects target,
11. execution planner computes required trade delta,
12. orders are emitted if thresholds are met,
13. portfolio and realized state are updated.

## 7.2 End-to-end training pipeline

During training:

1. historical event stream is replayed,
2. world model outputs are either precomputed or recomputed causally,
3. environment constructs observation,
4. policy produces action,
5. simulator applies constraints and costs,
6. environment advances one step,
7. reward is computed,
8. transition is stored,
9. RL update is performed according to the chosen algorithm,
10. evaluation is run in walk-forward segments.

---

# 8. Enumerated Object Model

This section defines the core objects of the policy system.

## 8.1 Object 1 — `ForecastPacket`

### Purpose
Represents the probabilistic future belief generated by the world model.

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

### Semantics
- `q_low[h]` = lower quantile forecast for horizon `h`
- `q_med[h]` = median forecast for horizon `h`
- `q_high[h]` = upper quantile forecast for horizon `h`
- `interval_width[h] = q_high[h] - q_low[h]`
- `regime_vector` = soft probabilities over market regimes
- `confidence_score` = scalar or horizon-wise confidence metric
- `ensemble_variance` = disagreement across model members
- `ood_score` = out-of-distribution score or anomaly score

### Example structural contract
```python
@dataclass
class ForecastPacket:
    timestamp: Any
    horizons: list[int]
    q_low: ArrayLike          # [H]
    q_med: ArrayLike          # [H]
    q_high: ArrayLike         # [H]
    interval_width: ArrayLike # [H]
    regime_vector: ArrayLike  # [F_regime]
    confidence_score: ArrayLike | float
    ensemble_variance: ArrayLike
    ood_score: float
    forecast_diagnostics: dict
```

---

## 8.2 Object 2 — `PortfolioState`

### Purpose
Represents the current capital, holdings, and PnL state visible to the policy.

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

## 8.3 Object 3 — `ExecutionState`

### Purpose
Represents immediate execution-relevant market conditions.

### Required fields
1. `mid_price`
2. `spread`
3. `estimated_slippage`
4. `estimated_fee_rate`
5. `available_liquidity_score`
6. `latency_proxy`
7. `volatility_proxy`
8. `order_book_imbalance` if available
9. `recent_trade_flow` if available

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

## 8.4 Object 4 — `RiskState`

### Purpose
Represents active hard constraints and policy envelopes.

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

## 8.5 Object 5 — `PolicyObservation`

### Purpose
Represents the complete state presented to the policy network.

### Components
1. world-model component,
2. portfolio component,
3. execution component,
4. risk component,
5. optional recent action history,
6. optional recent realized return history.

### Canonical form
The observation may be represented either as:

- a flat vector,
- a structured dictionary,
- or a grouped tensor bundle.

The grouping is preferable because it preserves semantic boundaries.

### Example contract
```python
@dataclass
class PolicyObservation:
    forecast_features: ArrayLike
    portfolio_features: ArrayLike
    execution_features: ArrayLike
    risk_features: ArrayLike
    history_features: ArrayLike | None
    metadata: dict
```

---

## 8.6 Object 6 — `PolicyAction`

### Purpose
Represents the raw output of the policy network prior to risk gating.

### Canonical first-version action types

#### Action Type A — target signed exposure
`a_t ∈ [-1, 1]`

Interpretation:
- `-1` = maximum allowed short
- `0` = flat
- `+1` = maximum allowed long

#### Action Type B — target exposure + aggressiveness
`a_t = [target_exposure, aggressiveness]`

Interpretation:
- first component determines desired signed position,
- second component determines urgency or allowable execution intensity.

#### Action Type C — target delta
`a_t ∈ [-δ_max, δ_max]`

Interpretation:
- directly proposes change in position rather than final target.

### Default recommendation
Use **Action Type A** for the first implementation.

### Example contract
```python
@dataclass
class PolicyAction:
    target_exposure: float
    action_diagnostics: dict
```

---

## 8.7 Object 7 — `TargetPosition`

### Purpose
Represents the desired post-trade position after action interpretation.

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

## 8.8 Object 8 — `ApprovedTarget`

### Purpose
Represents the risk-adjusted target after validation.

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

## 8.9 Object 9 — `ExecutionPlan`

### Purpose
Represents how the approved target difference should be converted into orders.

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

## 8.10 Object 10 — `TransitionRecord`

### Purpose
Represents one RL training transition.

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

# 9. Observation Construction Specification

## 9.1 Purpose

The observation constructor converts all upstream objects into a policy-consumable representation.

## 9.2 Forecast-derived feature groups

The observation must include, at minimum:

1. horizon-wise median returns,
2. horizon-wise lower quantiles,
3. horizon-wise upper quantiles,
4. horizon-wise interval widths,
5. horizon-wise normalized edge scores,
6. regime vector,
7. confidence score,
8. ensemble disagreement summary,
9. OOD score.

## 9.3 Derived forecast features

For each horizon `h`, compute optionally:

1. `edge_h = q_med[h]`
2. `risk_adj_edge_h = q_med[h] / (interval_width[h] + ε)`
3. `upside_h = max(q_high[h], 0)`
4. `downside_h = min(q_low[h], 0)`
5. `skew_h = q_high[h] - 2*q_med[h] + q_low[h]`
6. `sign_prob_proxy_h` from calibrated quantiles or ensemble votes
7. `term_structure_features` across horizons

These are not replacements for raw forecast quantiles; they are complementary abstractions.

## 9.4 Portfolio-derived features

Include at minimum:

1. current signed position fraction,
2. current leverage,
3. unrealized PnL,
4. realized PnL,
5. time in position,
6. time since last trade,
7. current drawdown state,
8. recent turnover.

## 9.5 Execution-derived features

Include at minimum:

1. spread,
2. slippage estimate,
3. fee estimate,
4. liquidity score,
5. recent realized volatility,
6. optional order-book features.

## 9.6 Risk-derived features

Include at minimum:

1. max allowed position fraction,
2. remaining position delta budget,
3. cooldown active flag,
4. kill switch flag,
5. remaining daily loss budget.

## 9.7 Observation constructor interface

```python
class PolicyObservationBuilder:
    def build(
        self,
        forecast_packet: ForecastPacket,
        portfolio_state: PortfolioState,
        execution_state: ExecutionState,
        risk_state: RiskState,
        history_context: dict | None = None,
    ) -> PolicyObservation:
        ...
```

---

# 10. Policy Network Specification

## 10.1 Purpose

The policy network maps structured policy observations to action proposals.

## 10.2 First-version architecture

The first implementation should be a **multi-branch MLP policy encoder** rather than a monolithic flat MLP.

### Rationale
The input is semantically structured:
- forecast features,
- portfolio features,
- execution features,
- risk features.

Each group should first receive its own encoder, then the encoded groups should be fused.

## 10.3 Canonical architecture

### Stage 1 — group encoders
1. forecast encoder,
2. portfolio encoder,
3. execution encoder,
4. risk encoder.

### Stage 2 — fused latent
Concatenate encoded group outputs into a joint decision latent.

### Stage 3 — policy head
Project fused latent into action parameters.

### Stage 4 — optional value head
For actor-critic methods, compute value estimate from the same or similar latent.

## 10.4 Interface

```python
class PolicyNetwork:
    def forward(self, obs: PolicyObservation) -> dict:
        """
        returns:
            {
                "action_params": ...,
                "value": ...,
                "policy_latent": ...,
                "diagnostics": ...
            }
        """
        ...
```

## 10.5 Actor outputs

For continuous actions, the actor should output parameters of an action distribution, such as:

1. mean target exposure,
2. scale / standard deviation,
3. optional aggressiveness parameter.

During training, action may be sampled.  
During inference, action may be mean, clipped mean, or another deterministic projection.

---

# 11. Action Semantics

## 11.1 Canonical action space

Use continuous target exposure:

`a_t = target_exposure_t ∈ [-1, 1]`

## 11.2 Interpretation

`target_fraction_t = a_t * max_abs_position_fraction_t`

If shorting is disabled:

`target_fraction_t = max(0, a_t) * max_abs_position_fraction_t`

If only long/flat is supported:

`a_t` may be constrained to `[0, 1]`.

## 11.3 Why target exposure is preferred

1. it is execution-agnostic,
2. it naturally supports partial sizing,
3. it reduces impulsive order behavior,
4. it aligns with portfolio-control interpretation,
5. it is compatible with deterministic risk clamping.

---

# 12. Action Projection Layer

## 12.1 Purpose

The raw policy action may lie outside admissible range or may conflict with risk rules.

## 12.2 Projection steps

1. clamp to numeric action bounds,
2. map to target position fraction,
3. enforce long/short permissions,
4. enforce leverage limit,
5. enforce per-step delta limit,
6. enforce cooldown rules,
7. enforce minimum meaningful change threshold.

## 12.3 Interface

```python
class ActionProjector:
    def project(
        self,
        action: PolicyAction,
        portfolio_state: PortfolioState,
        risk_state: RiskState,
    ) -> TargetPosition:
        ...
```

---

# 13. Risk Gate Specification

## 13.1 Purpose

The risk gate is a deterministic layer that validates and optionally modifies the target position.

## 13.2 Required checks

1. kill switch check,
2. OOD forecast check,
3. confidence threshold check,
4. interval width threshold check,
5. ensemble disagreement threshold check,
6. max absolute exposure check,
7. per-step exposure change check,
8. cooldown check,
9. drawdown check,
10. daily loss limit check,
11. volatility regime restriction check,
12. minimum order size check.

## 13.3 Gate outcomes

The gate may:

1. approve unchanged,
2. approve with clamp,
3. reject entirely,
4. convert to flat target.

## 13.4 Interface

```python
class RiskGate:
    def evaluate(
        self,
        target: TargetPosition,
        forecast_packet: ForecastPacket,
        portfolio_state: PortfolioState,
        execution_state: ExecutionState,
        risk_state: RiskState,
    ) -> ApprovedTarget:
        ...
```

---

# 14. Execution Planning Specification

## 14.1 Purpose

The policy does not create exchange orders directly.  
It creates a target.  
The execution planner computes what trade delta is needed.

## 14.2 Canonical computation

Let:

`current_fraction = portfolio_state.position_fraction`

`approved_fraction = approved_target.approved_target_fraction`

Then:

`required_delta_fraction = approved_fraction - current_fraction`

If:

`abs(required_delta_fraction) < min_effective_change`

then skip execution.

## 14.3 Planner responsibilities

1. compute target delta,
2. evaluate economic significance,
3. assign urgency,
4. choose execution mode,
5. split delta into child instructions if needed.

## 14.4 Interface

```python
class ExecutionPlanner:
    def plan(
        self,
        approved_target: ApprovedTarget,
        portfolio_state: PortfolioState,
        execution_state: ExecutionState,
    ) -> ExecutionPlan:
        ...
```

---

# 15. RL Environment Specification

## 15.1 Purpose

Defines the environment in which the policy is trained.

## 15.2 Environment state

The environment must track:

1. event-time market observations,
2. world-model outputs,
3. portfolio state,
4. execution state,
5. risk state,
6. current episode statistics.

## 15.3 Environment step

At each step:

1. receive action,
2. project action,
3. risk-gate action,
4. plan execution,
5. apply simulated fills and costs,
6. update position and cash,
7. advance market to next step,
8. compute reward,
9. assemble next observation,
10. return transition.

## 15.4 Interface

```python
class TradingPolicyEnvironment:
    def reset(self) -> PolicyObservation:
        ...

    def step(self, action: PolicyAction):
        """
        returns:
            next_observation,
            reward,
            done,
            info
        """
        ...
```

---

# 16. Cost Model Specification

## 16.1 Purpose

Prevents the policy from learning unrealistic churn.

## 16.2 Cost components

The training simulator must model:

1. explicit trading fees,
2. spread crossing cost,
3. slippage cost,
4. optional market impact proxy,
5. optional borrow / funding cost if shorting or leveraged products exist.

## 16.3 Canonical transaction-cost formula

A simple first version:

`cost_t = fee_rate_t * traded_notional_t + spread_component_t + slippage_component_t`

Where:

- `spread_component_t` may scale with aggressiveness and market side,
- `slippage_component_t` may scale with trade size relative to liquidity.

---

# 17. Reward Function Specification

## 17.1 Canonical reward decomposition

A recommended reward is:

`r_t = pnl_net_t - λ_turn * turnover_t - λ_dd * dd_increment_t - λ_risk * risk_violation_proxy_t - λ_churn * sign_flip_penalty_t`

Where:

1. `pnl_net_t` = realized + unrealized mark-to-market equity change after costs,
2. `turnover_t` penalizes excessive trading,
3. `dd_increment_t` penalizes worsening drawdown,
4. `risk_violation_proxy_t` penalizes behavior near hard boundaries,
5. `sign_flip_penalty_t` penalizes unnecessary oscillation.

## 17.2 Optional shaped rewards

Additional optional terms:

1. abstention reward when uncertainty is high and staying out is correct,
2. reward for maintaining profitable exposure through noise,
3. penalty for trading against forecast-confidence filters,
4. penalty for action under OOD conditions.

---

# 18. Value Function Specification

## 18.1 Purpose

For actor-critic methods, estimate expected future return from current observation.

## 18.2 Value input

The value network consumes the same observation as the actor, optionally with action for Q-style methods.

## 18.3 Output

`V_ψ(S_t)` or `Q_ψ(S_t, A_t)`

depending on the chosen RL algorithm.

---

# 19. Policy Algorithm Abstraction

## 19.1 Purpose

The spec should support multiple RL algorithms without changing the environment or object model.

## 19.2 Candidate algorithms

The policy architecture should be compatible with:

1. PPO-style actor-critic,
2. SAC-style stochastic actor-critic,
3. TD3-style deterministic actor-critic,
4. other continuous-control algorithms.

## 19.3 Abstraction interface

```python
class RLPolicyAlgorithm:
    def select_action(self, obs: PolicyObservation, deterministic: bool) -> PolicyAction:
        ...
    def update(self, batch) -> dict:
        ...
    def save(self, path): ...
    def load(self, path): ...
```

The environment and object contracts must not depend on a specific algorithm family.

---

# 20. Training Data Strategy

## 20.1 World-model integration modes

The policy training stack should support two modes:

### Mode A — precomputed forecast packets
The forecaster is run causally over the historical stream in advance, and its outputs are stored.

### Mode B — live coupled replay
The forecaster is executed during environment replay.

## 20.2 Recommended default
Use **Mode A** for the first implementation because it simplifies reproducibility and reduces coupling.

---

# 21. Episode Construction

## 21.1 Purpose

Defines how training trajectories are cut.

## 21.2 Episode options

1. fixed-length rolling episodes,
2. calendar-bounded episodes,
3. regime-bounded episodes,
4. continuous replay with truncation.

## 21.3 Recommended first implementation

Use **fixed-length rolling episodes** with resettable portfolio state and strict causal progression.

---

# 22. Policy Evaluation Specification

## 22.1 Primary evaluation metrics

The policy must be evaluated with:

1. total return,
2. annualized or period-adjusted Sharpe-like metric,
3. Sortino-like metric,
4. max drawdown,
5. turnover,
6. trade count,
7. win rate,
8. average holding time,
9. average cost per trade,
10. exposure utilization,
11. time-in-market,
12. policy abstention rate.

## 22.2 Calibration-aware metrics

Because the policy depends on forecasts, also evaluate:

1. performance when forecast confidence is high,
2. performance when interval width is low,
3. performance by regime,
4. performance under OOD conditions,
5. performance under widened cost stress.

## 22.3 Stress tests

The evaluation stack should support:

1. increased fee stress,
2. increased slippage stress,
3. latency perturbation,
4. volatility shock periods,
5. low-liquidity periods,
6. outlier market segments.

---

# 23. Walk-Forward Training and Validation

## 23.1 Purpose

Avoids leakage and improves realism.

## 23.2 Procedure

For each walk-forward segment:

1. fit or refresh forecaster upstream on training window,
2. generate causal forecast packets,
3. train policy on training window,
4. tune policy on validation window,
5. freeze policy,
6. evaluate on next unseen test window,
7. roll forward and repeat.

## 23.3 Why this is mandatory

A policy trained on one market regime may exploit patterns that do not persist.  
Walk-forward evaluation is therefore part of the core method, not an optional extra.

---

# 24. Online Inference Semantics

## 24.1 Runtime sequence

At each decision event:

1. receive latest forecast packet,
2. read current portfolio state,
3. read execution state,
4. read risk state,
5. build policy observation,
6. run policy forward pass,
7. decode action,
8. project to target,
9. run risk gate,
10. build execution plan,
11. execute only if economically meaningful.

## 24.2 Decision frequency

The policy may run every base timestep.  
Execution is not mandatory every timestep.  
The system may infer continuously but only trade when the approved target differs enough from the current position.

---

# 25. Abstention Logic

## 25.1 Purpose

The system must be allowed to do nothing.

## 25.2 Abstention conditions

The system may abstain when:

1. confidence is too low,
2. interval width is too wide,
3. OOD score is too high,
4. ensemble disagreement is too high,
5. action delta is too small,
6. cost estimate dominates expected edge,
7. cooldown is active,
8. risk state forbids new exposure.

## 25.3 Implementation form

Abstention may be represented as:

- target exposure equal to current exposure,
- or target exposure equal to flat,
- depending on policy and risk rules.

---

# 26. Serialization Scope

The following objects must be serializable independently:

1. policy network weights,
2. value network weights,
3. optimizer state,
4. replay buffer or trajectory store metadata where relevant,
5. observation normalizer state,
6. action scaler state,
7. policy configuration,
8. risk configuration,
9. reward configuration,
10. evaluation metadata.

---

# 27. Package Layout

A framework-neutral logical package layout is:

```text
policy_model/
  config/
    schema.py
    defaults.yaml
  objects/
    forecast_packet.py
    portfolio_state.py
    execution_state.py
    risk_state.py
    policy_observation.py
    policy_action.py
    target_position.py
    approved_target.py
    execution_plan.py
    transition_record.py
  observation/
    builder.py
    transforms.py
    normalization.py
  policy/
    encoders.py
    actor.py
    critic.py
    policy_network.py
    action_projection.py
  risk/
    gate.py
    rules.py
  env/
    environment.py
    simulator.py
    costs.py
    rewards.py
  training/
    trainer.py
    rollout.py
    buffers.py
    evaluation.py
    walkforward.py
  inference/
    service.py
    pipeline.py
  utils/
    logging.py
    serialization.py
```

---

# 28. Top-Level Composition

## 28.1 Canonical composition

```python
class PolicySystem:
    def __init__(
        self,
        observation_builder,
        policy_algorithm,
        action_projector,
        risk_gate,
        execution_planner,
    ):
        self.observation_builder = observation_builder
        self.policy_algorithm = policy_algorithm
        self.action_projector = action_projector
        self.risk_gate = risk_gate
        self.execution_planner = execution_planner

    def decide(
        self,
        forecast_packet,
        portfolio_state,
        execution_state,
        risk_state,
        history_context=None,
        deterministic=True,
    ):
        obs = self.observation_builder.build(
            forecast_packet=forecast_packet,
            portfolio_state=portfolio_state,
            execution_state=execution_state,
            risk_state=risk_state,
            history_context=history_context,
        )

        action = self.policy_algorithm.select_action(obs, deterministic=deterministic)
        target = self.action_projector.project(action, portfolio_state, risk_state)
        approved = self.risk_gate.evaluate(
            target=target,
            forecast_packet=forecast_packet,
            portfolio_state=portfolio_state,
            execution_state=execution_state,
            risk_state=risk_state,
        )
        plan = self.execution_planner.plan(
            approved_target=approved,
            portfolio_state=portfolio_state,
            execution_state=execution_state,
        )

        return {
            "observation": obs,
            "action": action,
            "target": target,
            "approved_target": approved,
            "execution_plan": plan,
        }
```

---

# 29. Concrete Canonical Interfaces

## 29.1 `PolicyObservationBuilder`

```python
class PolicyObservationBuilder:
    def build(
        self,
        forecast_packet: ForecastPacket,
        portfolio_state: PortfolioState,
        execution_state: ExecutionState,
        risk_state: RiskState,
        history_context: dict | None = None,
    ) -> PolicyObservation:
        ...
```

## 29.2 `PolicyAlgorithm`

```python
class PolicyAlgorithm:
    def select_action(
        self,
        obs: PolicyObservation,
        deterministic: bool = True,
    ) -> PolicyAction:
        ...

    def update(self, batch) -> dict:
        ...
```

## 29.3 `ActionProjector`

```python
class ActionProjector:
    def project(
        self,
        action: PolicyAction,
        portfolio_state: PortfolioState,
        risk_state: RiskState,
    ) -> TargetPosition:
        ...
```

## 29.4 `RiskGate`

```python
class RiskGate:
    def evaluate(
        self,
        target: TargetPosition,
        forecast_packet: ForecastPacket,
        portfolio_state: PortfolioState,
        execution_state: ExecutionState,
        risk_state: RiskState,
    ) -> ApprovedTarget:
        ...
```

## 29.5 `ExecutionPlanner`

```python
class ExecutionPlanner:
    def plan(
        self,
        approved_target: ApprovedTarget,
        portfolio_state: PortfolioState,
        execution_state: ExecutionState,
    ) -> ExecutionPlan:
        ...
```

## 29.6 `TradingPolicyEnvironment`

```python
class TradingPolicyEnvironment:
    def reset(self) -> PolicyObservation:
        ...

    def step(self, action: PolicyAction):
        ...
```

---

# 30. Concrete Decision Semantics

The first production-style interpretation should be:

1. the policy outputs a target signed exposure in normalized space,
2. normalized exposure is mapped to allowed capital fraction,
3. risk gate clamps to valid range,
4. execution planner compares approved target vs current position,
5. trade occurs only if the difference exceeds an economic threshold.

This means the policy is fundamentally a **position controller**.

---

# 31. Minimal Reference Pseudocode

```python
forecast_packet = world_model.infer(current_market_context)
portfolio_state = portfolio_service.read()
execution_state = execution_state_builder.build(current_market_context)
risk_state = risk_service.read()

obs = observation_builder.build(
    forecast_packet=forecast_packet,
    portfolio_state=portfolio_state,
    execution_state=execution_state,
    risk_state=risk_state,
)

action = policy_algorithm.select_action(obs, deterministic=True)

target = action_projector.project(
    action=action,
    portfolio_state=portfolio_state,
    risk_state=risk_state,
)

approved = risk_gate.evaluate(
    target=target,
    forecast_packet=forecast_packet,
    portfolio_state=portfolio_state,
    execution_state=execution_state,
    risk_state=risk_state,
)

plan = execution_planner.plan(
    approved_target=approved,
    portfolio_state=portfolio_state,
    execution_state=execution_state,
)

if not plan.skip_execution:
    execution_adapter.execute(plan)
```

---

# 32. Canonical First-Version Definition

The canonical first-version policy model defined by this document is:

**a forecast-conditioned, continuous-action reinforcement-learning policy that consumes probabilistic multi-horizon forecast packets, portfolio state, execution state, and risk state, and returns a target signed exposure which is then deterministically projected, risk-gated, and execution-planned before orders are emitted.**

In short form:

**Forecast Packet + Portfolio State + Execution State + Risk State → Policy Observation → RL Policy → Target Exposure → Risk Gate → Execution Plan**

---

# 33. Recommended First-Version Defaults

For the concrete first version, I recommend:

1. **continuous target exposure action** in `[-1, 1]`,
2. **multi-branch policy encoder** with separate forecast / portfolio / execution / risk encoders,
3. **actor-critic structure**,
4. **deterministic risk gate outside the actor**,
5. **net-PnL-minus-costs reward with drawdown and churn penalties**,
6. **walk-forward training**,
7. **precomputed causal forecast packets**,
8. **abstention by no-change target when confidence is weak**.

---

# 34. Most Important Non-Negotiable Rule

The most important design rule is:

**the policy does not decide how to place orders; it decides what exposure should be held.**

That single separation keeps the architecture clean, testable, and extensible.
