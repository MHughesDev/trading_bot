# APEX — Canonical Configuration Specification v1.0

**Document Type**: Configuration Specification  
**Scope**: System-wide thresholds, weights, flags, penalties, and behavior toggles  
**Version**: 1.0  
**Date**: April 2026  
**Status**: Build-ready draft  
**Parent Spec**: APEX Unified Full-System Master Spec v2.0

---

## 1. Purpose

This document defines the **canonical configuration model** for APEX.

It exists to ensure:
- thresholds are explicit
- weights are versioned
- flags are auditable
- behavior changes are replayable
- no hidden “magic constants” exist in code

This document is:
- implementation-agnostic
- format-agnostic
- storage-agnostic

It does **not** require:
- a particular config file format
- a specific configuration library
- a specific database

---

## 2. Configuration Design Principles

1. Every configurable behavior must have a stable name
2. Every threshold must have a clearly documented meaning
3. Every weight set must be versioned
4. Config changes must be replay-compatible
5. No config may silently change semantics across versions
6. Every config must have safe defaults
7. No config should allow obviously unsafe behavior without explicit operator intent

---

## 3. Configuration Domains

APEX configuration is divided into the following domains:

1. Global metadata and versioning
2. Signal confidence and decay
3. State / safety / degradation
4. Regime and transition logic
5. Forecast calibration
6. Trigger logic
7. Auction scoring and selection
8. Risk, sizing, and portfolio constraints
9. Execution logic
10. Memory and adaptation
11. Carry sleeve
12. Monitoring, alerts, and audit behavior
13. Replay / simulation behavior
14. Feature family enablement flags

---

## 4. Global Metadata and Versioning

### Required Fields
- `config_version`
- `config_name`
- `created_at`
- `created_by`
- `parent_config_version` (optional)
- `environment_scope` (e.g., research / sim / shadow / live)
- `notes`
- `enabled_feature_families`

### Rules
- every production run must reference an immutable config version
- every replay must reference the exact config version used
- changes must be diffable

---

## 5. Signal Confidence and Decay Configuration

### 5.1 Per-Signal Decay Parameters
Each signal family must support:
- `base_confidence_floor`
- `base_confidence_cap`
- `freshness_floor`
- `freshness_cap`
- `decay_lambda`
- `latency_penalty_weight`
- `reliability_penalty_weight`

### 5.2 Signal Families Requiring Independent Config
At minimum:
- `market_microstructure`
- `funding`
- `open_interest`
- `basis`
- `cross_exchange_divergence`
- `liquidation_structure`
- `options_context`
- `stablecoin_flow_proxy`
- `execution_feedback`
- `novelty`
- `heat_components`

### 5.3 Example Logical Structure

```text
signal_confidence:
  funding:
    base_confidence_floor
    base_confidence_cap
    freshness_floor
    decay_lambda
    latency_penalty_weight
    reliability_penalty_weight
```

### 5.4 Guardrails
- no confidence floor may exceed its cap
- decay lambdas must be non-negative
- stale but core signals should degrade, not necessarily hard-fail

---

## 6. State / Safety / Degradation Configuration

### 6.1 Safety Thresholds
- `novelty_warning_threshold`
- `novelty_defensive_threshold`
- `novelty_no_trade_threshold`
- `exchange_risk_defensive_threshold`
- `exchange_risk_no_trade_threshold`
- `volatility_circuit_breaker_threshold`
- `liquidity_collapse_threshold`
- `spread_stress_threshold_bps`

### 6.2 Heat Score Configuration
- `heat_component_weights`
  - funding extremity weight
  - liquidation proximity weight
  - OI fragility weight
  - divergence weight
  - volatility stress weight
  - execution stress weight
- `heat_reduced_threshold`
- `heat_defensive_threshold`
- `heat_no_trade_threshold` (optional)

### 6.3 Reflexivity Configuration
- `reflexivity_component_weights`
- `reflexivity_warning_threshold`
- `reflexivity_ceiling_threshold`
- `reflexivity_hard_ceiling_multiplier`

### 6.4 Degradation Multipliers
For each degradation state:
- `size_multiplier`
- `candidate_count_multiplier`
- `confidence_threshold_multiplier`
- `trigger_threshold_multiplier`
- `execution_aggression_multiplier`

Required states:
- `normal`
- `reduced`
- `defensive`
- `no_trade`

### 6.5 Weekend / Low-Liquidity Throttle
- `weekend_mode_enabled`
- `weekend_size_multiplier`
- `weekend_trigger_multiplier`
- `low_liquidity_size_multiplier`
- `low_liquidity_execution_penalty`

---

## 7. Regime and Transition Logic Configuration

### 7.1 Regime Probability Handling
- `regime_prob_floor`
- `regime_prob_smoothing`
- `regime_confidence_method`
- `regime_confidence_floor`

### 7.2 Transition Probability Configuration
- `transition_weight_regime_confidence_inverse`
- `transition_weight_volatility_shift`
- `transition_weight_microstructure_disagreement`
- `transition_weight_structural_disagreement`
- `transition_warning_threshold`
- `transition_guard_threshold`

### 7.3 Transition Guard Controls
- `transition_size_multiplier`
- `transition_trigger_multiplier`
- `transition_confidence_floor`
- `transition_candidate_penalty`

---

## 8. Forecast Calibration Configuration

### 8.1 Model Role Weights
- `weight_trend_model`
- `weight_range_model`
- `weight_volatility_model`

### 8.2 Outcome Calibration
- `epsilon_default_min`
- `epsilon_default_max`
- `epsilon_stable_max`
- `epsilon_half_life_hours`
- `epsilon_reset_on_regime_shift`
- `epsilon_reset_on_novelty`

### 8.3 Model Agreement / Overlap
- `model_agreement_weight`
- `model_overlap_penalty_weight`
- `model_overlap_penalty_floor`
- `model_overlap_penalty_cap`

### 8.4 OI Structure Contribution
Separate configurable contributions for:
- `healthy_trend`
- `fragile_buildup`
- `squeeze_potential`
- `deleveraging`

---

## 9. Trigger Configuration

### 9.1 Setup Stage
- `setup_threshold`
- `setup_weight_asymmetry`
- `setup_weight_state_alignment`
- `setup_weight_confidence`
- `setup_penalty_heat`
- `setup_penalty_novelty`
- `setup_execution_floor`

### 9.2 Pre-Trigger Stage
- `pretrigger_threshold`
- `pretrigger_weight_imbalance_shift`
- `pretrigger_weight_volume_expansion`
- `pretrigger_weight_tightening`
- `pretrigger_weight_structural_pressure`
- `pretrigger_freshness_floor`

### 9.3 Confirmed Trigger Stage
- `trigger_threshold`
- `trigger_weight_imbalance_spike`
- `trigger_weight_volume_burst`
- `trigger_weight_structure_break`
- `trigger_execution_floor`

### 9.4 Trigger Confidence
- `trigger_confidence_weight_structural`
- `trigger_confidence_weight_market`
- `trigger_confidence_weight_execution`
- `trigger_confidence_weight_decay`

### 9.5 Missed Move Acceptance
- `entry_extension_limit`
- `minimum_remaining_edge`
- `late_trigger_penalty`
- `missed_move_suppress_enabled`

### 9.6 Trigger Lifecycle
- `setup_max_lifetime_seconds`
- `pretrigger_max_lifetime_seconds`
- `trigger_context_expiry_seconds`

---

## 10. Auction Scoring and Selection Configuration

### 10.1 Positive Score Weights
- `auction_weight_asymmetry`
- `auction_weight_state_alignment`
- `auction_weight_confidence`
- `auction_weight_trigger`
- `auction_weight_execution`
- `auction_weight_oi_structure`
- `auction_weight_liquidation_opportunity`

### 10.2 Negative Score Weights
- `auction_penalty_diversification`
- `auction_penalty_model_overlap`
- `auction_penalty_false_positive_memory`
- `auction_penalty_degradation`
- `auction_penalty_edge_budget`
- `auction_penalty_concentration`

### 10.3 Auction Constraints
- `top_n_limit`
- `top_notional_limit`
- `per_instrument_candidate_limit`
- `per_thesis_candidate_limit`
- `minimum_auction_score`
- `minimum_execution_confidence`
- `minimum_trigger_confidence`

### 10.4 Diversification Configuration
- `diversification_weight_correlation`
- `diversification_weight_thesis_overlap`
- `diversification_weight_liquidation_overlap`
- `correlation_penalty_threshold`
- `same_thesis_penalty_enabled`

### 10.5 Edge Budget Configuration
- `edge_budget_weight_heat`
- `edge_budget_weight_concentration`
- `edge_budget_weight_overlap`
- `edge_budget_weight_confidence_adjusted_notional`
- `edge_budget_soft_threshold`
- `edge_budget_hard_threshold`

---

## 11. Risk, Sizing, and Portfolio Constraint Configuration

### 11.1 Size Controls
- `base_size_floor`
- `base_size_cap`
- `quantile_asymmetry_boost_cap`
- `trigger_confidence_size_multiplier_cap`
- `execution_confidence_size_multiplier_floor`

### 11.2 Position Inertia
- `max_position_delta_per_interval`
- `position_inertia_penalty_weight`

### 11.3 Exposure Constraints
- `max_gross_exposure`
- `max_net_exposure`
- `max_per_instrument_exposure`
- `max_per_thesis_exposure`

### 11.4 Correlation Constraints
- `correlation_group_threshold`
- `correlation_hard_limit_threshold`
- `correlation_penalty_weight`

### 11.5 Drawdown / Portfolio Heat
- `drawdown_warning_threshold`
- `drawdown_defensive_threshold`
- `drawdown_size_multiplier_warning`
- `drawdown_size_multiplier_defensive`

### 11.6 CVaR / Tail Controls
- `cvar_enabled`
- `cvar_limit`
- `tail_risk_penalty_weight`

### 11.7 Prohibited Sizing Flags
- `allow_kelly = false`
- `allow_streak_boost = false`
- `allow_fast_outcome_leverage_repricing = false`

---

## 12. Execution Logic Configuration

### 12.1 Execution Confidence
- `execution_weight_depth_quality`
- `execution_weight_spread_quality`
- `execution_weight_venue_quality`
- `execution_weight_latency_quality`
- `execution_weight_realized_slippage_quality`

### 12.2 Order Style Thresholds
- `high_confidence_passive_threshold`
- `medium_confidence_staggered_threshold`
- `aggressive_urgency_floor`
- `passive_spread_limit_bps`

### 12.3 Stress Mode
- `execution_stress_volatility_threshold`
- `execution_stress_spread_threshold_bps`
- `execution_stress_heat_threshold`
- `execution_stress_venue_quality_floor`
- `stress_mode_size_multiplier`
- `stress_mode_aggression_multiplier`

### 12.4 Worst-Case Edge Heuristic
- `minimum_tradeable_edge`
- `worst_case_slippage_multiplier`
- `adverse_fill_penalty_weight`
- `spread_risk_penalty_weight`

### 12.5 Partial Fill Handling
- `min_remaining_fraction`
- `partial_fill_problem_threshold`
- `low_execution_floor`
- `max_cancel_replace_count`

---

## 13. Memory and Adaptation Configuration

### 13.1 False Positive Memory
- `false_positive_memory_enabled`
- `false_positive_penalty_weight`
- `false_positive_decay_hours`
- `false_positive_similarity_threshold`

### 13.2 Opportunity Cost Tracking
- `opportunity_cost_tracking_enabled`
- `opportunity_cost_review_window_hours`
- `opportunity_cost_min_quality_threshold`

### 13.3 Drift Detection
- `drift_detection_enabled`
- `feature_drift_threshold`
- `output_drift_threshold`
- `realized_vs_theoretical_divergence_threshold`

### 13.4 Shadow Logic
- `shadow_mode_enabled`
- `shadow_comparison_window`
- `promotion_requires_review = true`

### 13.5 Retraining / Recalibration
- `slow_retrain_enabled`
- `retrain_window_schedule`
- `holdout_validation_required = true`

---

## 14. Carry Sleeve Configuration

- `carry_enabled`
- `carry_activation_requires_directional_neutrality`
- `carry_max_exposure`
- `carry_funding_threshold`
- `carry_independent_risk_multiplier`
- `carry_attribution_isolation_required = true`

---

## 15. Monitoring, Alerts, and Audit Configuration

### 15.1 Metrics Toggles
- `log_state_transitions`
- `log_trigger_progression`
- `log_candidate_scores`
- `log_suppressions`
- `log_execution_erosion`
- `log_false_positive_memory`

### 15.2 Alert Thresholds
- stale data alert thresholds
- high heat alert
- prolonged defensive state alert
- excessive suppression rate alert
- execution erosion alert
- drift alert

### 15.3 Audit Controls
- `decision_replay_required = true`
- `log_reason_codes_required = true`
- `log_config_version_required = true`

---

## 16. Replay / Simulation Configuration Hooks

To keep live and replay aligned, configuration must also support:
- `replay_execution_mode`
- `replay_slippage_profile`
- `replay_partial_fill_profile`
- `replay_stale_data_injection_enabled`
- `replay_trigger_debug_enabled`

---

## 17. Validation Rules

The configuration system must validate:

- thresholds are within meaningful ranges
- floors do not exceed caps
- multiplier values are non-negative
- enum values are valid
- required domains are present
- prohibited flags remain disabled unless explicitly overridden in non-production scopes

Validation failures should be categorized as:
- `hard_invalid`
- `unsafe_but_parseable`
- `deprecated`
- `missing_optional`

---

## 18. Safe Defaults

Every configuration domain must provide safe defaults that:
- do not allow unbounded size
- do not disable all protections
- do not assume perfect execution
- do not require optional exotic features to function

---

## 19. Versioning and Migration

### Required Metadata
- `schema_version`
- `config_version`
- `parent_config_version`
- `migration_notes`

### Rules
- any incompatible config change must bump version
- replay tooling must know which config version to load
- changed semantics must be documented

---

## 20. Acceptance Criteria

The canonical configuration spec is acceptable when:
1. all material behaviors are externally configurable
2. all production runs are attributable to a config version
3. replay and live can use identical logical config
4. unsafe combinations are catchable by validation
5. no magic constants remain hidden in implementation
