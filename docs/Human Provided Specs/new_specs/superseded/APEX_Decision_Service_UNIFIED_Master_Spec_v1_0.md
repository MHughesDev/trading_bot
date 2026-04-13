# APEX Decision Service — Unified Master Specification v1.0

**Document Type**: Unified Master Specification  
**Scope**: Decision Service only (architecture + formal code spec + feature schema + data contracts + runtime logic + audit checklist)  
**Version**: 1.0  
**Date**: April 2026  
**Status**: Final Unified Draft  
**System Context**: Crypto perpetual futures trading  
**Primary Objective**: Maximize long-run PnL by detecting leverage imbalances, waiting for valid triggers, selecting only the highest-quality trade opportunities, and exploiting forced flows while failing small and recovering fast.

---

# Table of Contents

1. Document Intent and Scope  
2. Core Philosophy and Final Converged Conclusions  
3. Decision Service Boundaries  
4. End-to-End Decision Pipeline  
5. Runtime Cadence Model  
6. External Interfaces  
7. Canonical Domain Objects  
8. Probabilistic Signal Framework  
9. Data Ingestion and Normalization Requirements  
10. Feature Schema & Data Contracts  
11. Layer 1 — State & Safety Engine  
12. Layer 2 — Forecasting / Structure Engine  
13. Layer 3 — Trigger Engine  
14. Layer 4 — Decision Policy and Trade Opportunity Auction  
15. Layer 5 — Risk & Portfolio Constraint Engine  
16. Execution Awareness Contract  
17. Memory, Feedback, and Slow Adaptation  
18. Degradation Hierarchy and Hard Overrides  
19. Configuration Domains  
20. Validation and Failure Handling Rules  
21. Observability, Logging, and Auditability  
22. Testing Requirements  
23. Implementation Roadmap  
24. Explicit Non-Goals  
25. Known Hard Risks  
26. Final Build Decision  
27. Unified Self-Audit Checklist  
28. Glossary

---

## 1. Document Intent and Scope

This document unifies the final converged conclusions from the full design debate into **one master specification** for the **APEX Decision Service**.

It combines:
- system identity and architecture
- formal service behavior
- feature schema
- data contracts
- decision logic
- trigger logic requirements
- risk behavior
- execution-awareness rules
- memory/adaptation behavior
- observability and testing requirements
- an explicit self-audit checklist

This document is intentionally:
- **decision-service centered**
- **product-agnostic**
- **dependency-agnostic**
- **transport-agnostic**
- **storage-agnostic**
- **language-agnostic**

It does **not** assume:
- a specific programming language
- any specific libraries or frameworks
- any specific message queue, database, exchange adapter, or broker API
- a full OMS/EMS architecture beyond minimal abstract interfaces

---

## 2. Core Philosophy and Final Converged Conclusions

### 2.1 Final System Identity

APEX is **not** a generic price predictor.

It is a:

> **leverage-flow exploitation decision service**

Its primary job is to:

```text
Detect leverage imbalance
→ assess asymmetry and fragility
→ wait for valid timing trigger
→ select the best opportunities
→ bound risk and execution damage
→ emit high-quality trade intents
```

### 2.2 Primary Edge Sources

The converged system relies primarily on:

1. **Liquidation pressure / forced flow**
2. **Open Interest (OI) structure**
3. **Funding pressure**
4. **Perp vs spot divergence / basis**
5. **Cross-exchange fragmentation / dislocation**
6. **Microstructure confirmation for timing**

### 2.3 What Was Rejected

The final converged design explicitly rejected or deferred:
- hierarchical RL for v1
- opponent modeling as a dedicated core subsystem
- Kelly sizing variants
- streak-based size increases
- fast performance-chasing adaptation
- fully autonomous per-trade learning
- heavyweight sentiment and whale heuristics as core signals
- complexity that improves “theory” but not realized PnL

### 2.4 Final Design Principles

1. **Leverage-flow exploitation over generic direction prediction**
2. **State drives structure**
3. **Outcomes only whisper for calibration**
4. **Every signal is uncertain**
5. **Timing is gated, not assumed**
6. **Trade quality beats trade count**
7. **Execution realism is non-negotiable**
8. **Mitigations must improve realized PnL, not just theoretical safety**
9. **Low confidence usually reduces size rather than causing paralysis**
10. **Hard overrides exist only for true danger conditions**
11. **The system must remain mechanically testable and replayable**

---

## 3. Decision Service Boundaries

### 3.1 In Scope

The Decision Service is responsible for:

1. Receiving normalized market/state/feature snapshots
2. Applying signal confidence, freshness, and decay
3. Constructing current market state
4. Producing compact probabilistic structure outputs
5. Detecting valid multi-stage triggers
6. Generating candidate trades
7. Ranking candidates via a constrained auction
8. Applying risk, degradation, and safety constraints
9. Emitting trade intents, suppressions, or no-trade decisions
10. Updating internal decision-relevant memory:
   - false positive memory
   - opportunity cost tracking
   - trigger trust
   - execution trust
   - transition guard state
   - edge budgeting proxies

### 3.2 Out of Scope

The Decision Service is **not** responsible for:

- raw market data transport collection
- order placement
- exchange connectivity
- full portfolio accounting source of truth
- wallet/custody management
- exchange authentication
- model training infrastructure
- backtesting infrastructure
- UI/dashboard
- operator workflow orchestration
- capital allocation outside decision-time interfaces
- final PnL accounting system

### 3.3 Relationship to Other Services

The Decision Service assumes upstream/downstream components may exist, but they are abstracted as contracts:
- market snapshot provider
- structural signal provider
- safety/regime provider
- execution feedback provider
- trade intent sink
- decision log sink
- metrics sink

---

## 4. End-to-End Decision Pipeline

```text
Input Snapshots
→ Validation / Confidence Normalization
→ State Construction
→ Forecast / Structure Evaluation
→ Trigger Evaluation
→ Candidate Generation
→ Candidate Auction
→ Risk & Constraint Application
→ Trade Intent / Suppression / No-Trade Output
→ Memory and Feedback Update
```

### 4.1 Pipeline Summary in Plain Terms

1. Understand the market state
2. Estimate asymmetry, continuation, and fragility
3. Wait for a real trigger
4. Generate possible trades
5. Rank only the best opportunities
6. Apply hard safety and risk rules
7. Issue trade intent only if quality remains high
8. Update memory from what happened

---

## 5. Runtime Cadence Model

The service operates at multiple logical speeds.

### 5.1 Fast Safety / State Refresh
Typical cadence:
- every 1–5 seconds or event-driven

Used for:
- novelty/OOD
- degradation changes
- exchange risk
- spread/liquidity collapse
- heat score shifts

### 5.2 Structure / Forecast Refresh
Typical cadence:
- every 1–5 minutes

Used for:
- quantiles
- asymmetry
- continuation probability
- fragility
- OI structure
- funding/basis state

### 5.3 Trigger Evaluation
Typical cadence:
- every 1–30 seconds depending on market activity

### 5.4 Auction / Decision Emission
Typical cadence:
- event-driven after trigger confirmation
- or periodic scans over active instruments

### 5.5 Slow Adaptation / Memory Review
Typical cadence:
- hours to days

Used for:
- calibration review
- drift review
- false positive memory decay
- opportunity cost review
- shadow logic evaluation

---

## 6. External Interfaces

## 6.1 Input Interfaces

### 6.1.1 Market Snapshot Provider
Provides normalized market features:
- prices
- spread
- book imbalance
- depth
- volume burst
- volatility
- liquidity state
- freshness and reliability

### 6.1.2 Structural Signal Provider
Provides leverage-flow features:
- funding
- funding velocity
- OI
- OI delta
- basis
- cross-exchange divergence
- liquidation proximity and density
- cascade estimates
- options context when available
- freshness and reliability

### 6.1.3 Safety / Regime Provider
Provides:
- regime probabilities
- transition confidence
- novelty/OOD score
- degradation recommendation
- heat score
- exchange risk flags
- weekend/low-liquidity context

### 6.1.4 Execution Feedback Provider
Provides:
- expected vs realized slippage
- fill ratio
- fill latency
- venue quality
- execution anomalies

## 6.2 Output Interfaces

### 6.2.1 Trade Intent Sink
Consumes trade intents.

### 6.2.2 Decision Log Sink
Consumes decision records and replay data.

### 6.2.3 Decision Metrics Sink
Consumes operational metrics and analytics.

---

## 7. Canonical Domain Objects

## 7.1 SignalValue

```text
SignalValue
- name: string
- raw_value: float
- normalized_value: float
- confidence: float [0,1]
- freshness: float [0,1]
- decayed_confidence: float [0,1]
- effective_value: float
- source_count: integer
- reliability_score: float [0,1]
- timestamp: datetime
- metadata: map
```

## 7.2 MarketState

```text
MarketState
- timestamp: datetime
- regime_probabilities: map<string,float>
- regime_confidence: float
- transition_probability: float
- novelty_score: float
- volatility_state: enum
- liquidity_state: enum
- spread_state: enum
- weekend_mode: bool
- exchange_risk_level: enum
- degradation_level: enum
- crypto_heat_score: float
- reflexivity_score: float
- confidence_vector: map<string,float>
```

## 7.3 ForecastState

```text
ForecastState
- quantiles: map<string,float>   # P5/P25/P50/P75/P95
- volatility_forecast: float
- asymmetry_score: float
- continuation_probability: float
- fragility_score: float
- directional_bias: float
- model_agreement_score: float
- model_correlation_penalty: float
- calibration_weight: float
- oi_structure_class: enum
```

## 7.4 TriggerState

```text
TriggerState
- setup_valid: bool
- setup_score: float
- pretrigger_valid: bool
- pretrigger_score: float
- trigger_valid: bool
- trigger_type: enum
- trigger_strength: float [0,1]
- trigger_confidence: float [0,1]
- missed_move_flag: bool
- trigger_reason_codes: list<string>
```

## 7.5 CandidateTrade

```text
CandidateTrade
- candidate_id: string
- instrument_id: string
- side: enum(long, short, flat_reduction)
- thesis_type: enum(trend, squeeze, liquidation_exploitation, defensive_reduction, neutral)
- entry_style: enum(passive, aggressive, staggered)
- asymmetry_score: float
- state_alignment_score: float
- confidence_score: float
- trigger_score: float
- execution_confidence_score: float
- oi_structure_class: enum
- liquidation_opportunity_score: float
- diversification_penalty: float
- auction_score: float
- proposed_size_fraction: float
- hard_reject_reasons: list<string>
- soft_penalties: list<string>
```

## 7.6 TradeIntent

```text
TradeIntent
- intent_id: string
- timestamp: datetime
- instrument_id: string
- side: enum(long, short, reduce, flat)
- urgency: enum(low, medium, high)
- size_fraction: float
- preferred_execution_style: enum(passive, aggressive, staggered, twap)
- decision_confidence: float
- trigger_confidence: float
- execution_confidence: float
- degradation_level: enum
- max_slippage_tolerance_bps: float
- reason_codes: list<string>
```

## 7.7 DecisionRecord
Full replayable decision object containing:
- input snapshot ids
- effective signals
- forecast outputs
- trigger outputs
- candidates
- auction rankings
- final output
- config version references
- rejection/suppression reasons
- confidence and penalty values

---

## 8. Probabilistic Signal Framework

### 8.1 Universal Signal Rule

Every signal must be represented as:

```text
effective_signal = normalized_value × confidence × freshness
```

### 8.2 Decay Rule

For time-sensitive signals:

```text
decayed_confidence(t) = base_confidence × exp(-λ × age)
```

Where:
- `λ` is tuned by signal family
- age is signal age or time since last confirmation

### 8.3 Confidence Drivers

Confidence generally decreases when:
- latency rises
- reliability drops
- source consistency is poor
- exchange risk is elevated
- signal family is known to be weaker in current regime

### 8.4 Action Under Uncertainty Curve

The service must not freeze unnecessarily.

General rule:
- low confidence → reduce size and rank
- very low confidence + safety conflict → suppress
- hard override conditions → no-trade / defensive state

### 8.5 Soft vs Hard Data Handling

Missing or degraded data should usually:
- reduce confidence
- increase decay
- worsen rank
- shrink size

It should only hard-block action when:
- core safety inputs are invalid
- core price inputs are invalid
- exchange / novelty conditions are critical

---

## 9. Data Ingestion and Normalization Requirements

### 9.1 Input Philosophy
The Decision Service consumes **normalized inputs**, not raw exchange-native payloads.

### 9.2 Source Redundancy
Multi-source redundancy is allowed for:
- liquidation data
- OI
- funding
- basis / cross-exchange

Redundancy should improve confidence estimates, not create hard “consensus or reject” paralysis.

### 9.3 Freshness Classes

#### Fast Fields
Expected freshness:
- 1–5 seconds

Examples:
- best bid/ask
- spread
- imbalance
- depth
- volume burst

#### Medium Fields
Expected freshness:
- 10–60 seconds

Examples:
- basis
- OI delta
- cross-exchange divergence
- rolling execution state

#### Slow Fields
Expected freshness:
- 1–10 minutes

Examples:
- options context
- stablecoin flow proxies
- slower structural indicators

If freshness exceeds tolerances:
- lower confidence
- raise decay
- possibly raise degradation level if critical

### 9.4 Data Sanity Rule
Sanity layers must be **soft by default**:
- lower confidence
- mark anomaly
- preserve replayability
- avoid rejecting true chaos when chaos is the actual opportunity

---

## 10. Feature Schema & Data Contracts

## 10.1 Canonical Input Contract: Market Snapshot

### Required Fields

| Field | Type | Description |
|---|---|---|
| `schema_version` | string | Contract version |
| `snapshot_id` | string | Unique input identifier |
| `timestamp` | datetime | UTC timestamp |
| `instrument_id` | string | Canonical instrument id |
| `venue_group` | string | Venue/source family |
| `last_price` | float | Latest tradable price |
| `mid_price` | float | Midpoint price |
| `best_bid` | float | Best bid |
| `best_ask` | float | Best ask |
| `spread_bps` | float | Spread in basis points |
| `realized_vol_short` | float | Short-horizon realized volatility |
| `realized_vol_medium` | float | Medium-horizon realized volatility |
| `book_imbalance` | float | Normalized imbalance |
| `depth_near_touch` | float | Near-touch depth |
| `trade_volume_short` | float | Short-horizon trade volume |
| `volume_burst_score` | float | Abnormal volume score |
| `market_freshness` | float | Snapshot freshness |
| `market_reliability` | float | Snapshot reliability |
| `session_mode` | enum | `regular`, `weekend`, `low_liquidity`, `stressed` |

### Optional Fields
- `microprice`
- `depth_bid_1pct`
- `depth_ask_1pct`
- `trade_count_short`
- `price_return_short`
- `price_return_medium`
- `local_structure_break_score`
- `exchange_health_score`
- `source_latency_ms`

### Validation Rules
- `best_ask >= best_bid`
- `spread_bps >= 0`
- freshness/reliability in `[0,1]`
- `last_price > 0`

---

## 10.2 Canonical Input Contract: Structural Signal Snapshot

### Required Fields

| Field | Type | Description |
|---|---|---|
| `schema_version` | string | Contract version |
| `snapshot_id` | string | Snapshot id |
| `timestamp` | datetime | UTC timestamp |
| `instrument_id` | string | Instrument |
| `funding_rate` | float | Current funding |
| `funding_rate_zscore` | float | Standardized funding |
| `funding_velocity` | float | Funding rate-of-change |
| `open_interest` | float | Current OI |
| `open_interest_delta_short` | float | Short-horizon OI change |
| `basis_bps` | float | Basis in basis points |
| `cross_exchange_divergence` | float | Cross-venue divergence |
| `liquidation_proximity_long` | float | Proximity to long-side liquidation zone |
| `liquidation_proximity_short` | float | Proximity to short-side liquidation zone |
| `liquidation_cluster_density_long` | float | Long-side cluster density |
| `liquidation_cluster_density_short` | float | Short-side cluster density |
| `liquidation_data_confidence` | float | Confidence in liquidation structure |
| `signal_freshness_structural` | float | Freshness of structural bundle |
| `signal_reliability_structural` | float | Reliability of structural bundle |

### Optional / Conditional Fields
- `cascade_magnitude_estimate_long`
- `cascade_magnitude_estimate_short`
- `oi_concentration_score`
- `oi_price_structure_class`
- `perp_spot_divergence_score`
- `funding_cross_exchange_dispersion`
- `gex_score`
- `iv_skew_score`
- `options_freshness`
- `options_reliability`
- `stablecoin_flow_proxy`
- `exchange_leverage_skew_score`
- `signal_source_count`

### Contract Rules
- structural fields may be stale, but staleness must reduce confidence rather than break the cycle
- options-derived inputs are conditional and must not block the service if absent

---

## 10.3 Canonical Input Contract: Safety / Regime Snapshot

### Required Fields

| Field | Type | Description |
|---|---|---|
| `schema_version` | string | Contract version |
| `snapshot_id` | string | Snapshot id |
| `timestamp` | datetime | UTC timestamp |
| `instrument_id` | string | Instrument or scope |
| `regime_probabilities` | map<string,float> | Regime probabilities |
| `regime_confidence` | float | Confidence in regime estimate |
| `transition_probability` | float | Transition probability |
| `novelty_score` | float | OOD score |
| `crypto_heat_score` | float | Heat metric |
| `reflexivity_score` | float | Crowding metric |
| `degradation_level` | enum | `normal`, `reduced`, `defensive`, `no_trade` |
| `weekend_mode` | boolean | Weekend/low-liquidity flag |
| `exchange_risk_level` | enum | `low`, `elevated`, `high`, `critical` |

### Optional Fields
- `degradation_reason_codes`
- `volatility_circuit_breaker_active`
- `data_integrity_alert`
- `transition_guard_active`

---

## 10.4 Canonical Input Contract: Execution Feedback Snapshot

### Required Fields

| Field | Type | Description |
|---|---|---|
| `schema_version` | string | Contract version |
| `feedback_id` | string | Feedback id |
| `timestamp` | datetime | UTC timestamp |
| `instrument_id` | string | Instrument |
| `related_intent_id` | string | Trade intent id |
| `expected_fill_price` | float | Expected fill |
| `realized_fill_price` | float | Actual fill |
| `realized_slippage_bps` | float | Slippage in bps |
| `fill_ratio` | float | Filled fraction |
| `fill_latency_ms` | duration_ms | Fill latency |
| `execution_confidence_realized` | float | Realized execution quality |
| `venue_quality_score` | float | Venue quality |

### Optional Fields
- `partial_fill_flag`
- `cancel_replace_count`
- `order_style_used`
- `execution_stress_flag`
- `execution_anomaly_codes`

---

## 10.5 Canonical Internal Object: Decision Snapshot

This is the normalized object the service should build before trigger and auction logic.

Required contents:
- market snapshot
- structural snapshot
- safety snapshot
- effective signal map
- forecast outputs
- model agreement / overlap values
- execution confidence estimate
- false positive memory penalty
- edge budget proxy

---

## 10.6 Forecast / Structure Output Contract

Required fields:
- `p05`, `p25`, `p50`, `p75`, `p95`
- `volatility_forecast`
- `asymmetry_score`
- `continuation_probability`
- `fragility_score`
- `directional_bias`
- `model_agreement_score`
- `model_correlation_penalty`
- `calibration_weight`
- `oi_structure_class`

Allowed OI structure classes:
- `healthy_trend`
- `fragile_buildup`
- `squeeze_potential`
- `deleveraging`
- `unknown`

---

## 10.7 Trigger Contract

Required fields:
- `setup_valid`
- `setup_score`
- `pretrigger_valid`
- `pretrigger_score`
- `trigger_valid`
- `trigger_type`
- `trigger_strength`
- `trigger_confidence`
- `missed_move_flag`
- `trigger_reason_codes`

Allowed trigger types:
- `imbalance_spike`
- `volume_burst`
- `structure_break`
- `composite_confirmed`
- `none`

---

## 10.8 Candidate Trade Contract

Required fields:
- `candidate_id`
- `instrument_id`
- `side`
- `thesis_type`
- `entry_style`
- `asymmetry_score`
- `state_alignment_score`
- `confidence_score`
- `trigger_score`
- `execution_confidence_score`
- `oi_structure_class`
- `liquidation_opportunity_score`
- `diversification_penalty`
- `auction_score`
- `proposed_size_fraction`
- `hard_reject_reasons`
- `soft_penalties`

---

## 10.9 Output Contracts

### Trade Intent
Required fields:
- `intent_id`
- `timestamp`
- `instrument_id`
- `side`
- `urgency`
- `size_fraction`
- `preferred_execution_style`
- `decision_confidence`
- `trigger_confidence`
- `execution_confidence`
- `degradation_level`
- `max_slippage_tolerance_bps`
- `reason_codes`

### Suppression Event
Required fields:
- `event_id`
- `timestamp`
- `instrument_id`
- `suppression_type`
- `reason_codes`
- `blocked_candidate_id`
- `degradation_level`

### No-Trade Decision
Required fields:
- `event_id`
- `timestamp`
- `instrument_id`
- `no_trade_reason_codes`
- `state_summary`

### Safety Override Event
Required fields:
- `event_id`
- `timestamp`
- `override_type`
- `reason_codes`
- `affected_instruments`

### Decision Record
Must include:
- input snapshot ids
- effective signals
- state outputs
- forecast outputs
- trigger outputs
- candidates
- auction ranking
- selected output
- config versions
- suppression reasons
- override reasons

---

## 11. Layer 1 — State & Safety Engine

### 11.1 Purpose
Construct the current decision state and enforce hard overrides.

### 11.2 Inputs
- market features
- structural features
- safety/regime fields
- execution quality context
- session context

### 11.3 Outputs
- regime probabilities
- regime confidence
- transition probability
- novelty score
- degradation level
- reflexivity score
- crypto heat score
- override flags
- confidence vector

### 11.4 Mandatory Logic
- probabilistic regime handling
- novelty override support
- degradation transitions
- weekend / low-liquidity throttle
- exchange risk handling
- signal freshness and decay propagation

### 11.5 Crypto Heat Score
The heat score should incorporate, at minimum:
- funding extremes
- liquidation proximity
- OI concentration or fragility
- cross-exchange divergence
- volatility
- execution stress

---

## 12. Layer 2 — Forecasting / Structure Engine

### 12.1 Purpose
Estimate the structure of opportunity, not unconstrained price prophecy.

### 12.2 Logical Model Roles
The implementation should support three logical model families:
- trend-sensitive sequential
- range/chop-sensitive tabular
- volatility specialist

Exact algorithms are implementation-defined.

### 12.3 Required Outputs
- quantiles
- volatility
- asymmetry
- continuation probability
- fragility
- weak directional bias
- agreement score
- overlap penalty

### 12.4 Calibration Policy
Outcome-aware calibration:
- weak only
- epsilon in `0.03–0.05` typically
- hard max `0.08`
- half-life `12–24h`
- reset on regime shift or novelty

### 12.5 Liquidation Handling
Liquidation data must be:
- probabilistic
- decay-weighted
- confidence-weighted
- bounded by explicit uncertainty

### 12.6 OI Structure Logic
OI structure classification must directly affect:
- continuation probability
- asymmetry
- liquidation opportunity score
- risk sizing confidence

---

## 13. Layer 3 — Trigger Engine

### 13.1 Purpose
Solve the timing problem.

### 13.2 Trigger Minimalism
The trigger engine must stay:
- compact
- explicit
- backtestable
- replayable

### 13.3 Multi-Stage Design

#### Stage 1 — Setup
Requirements:
- sufficient asymmetry
- acceptable state alignment
- no blocking safety conflict

#### Stage 2 — Pre-Trigger
Evidence of pressure-building:
- imbalance shift
- tightening
- early volume expansion

#### Stage 3 — Confirmed Trigger
Mechanically defined event:
- imbalance spike
- volume burst
- break of local structure
- bounded composite trigger

### 13.4 Missed Move Acceptance
The engine must explicitly avoid chasing when:
- the move has already extended beyond acceptable entry quality
- trigger is late relative to asymmetry

### 13.5 Trigger Confidence Rules
Trigger confidence may affect:
- size
- ranking
- urgency

But only within bounded ranges.

---

## 14. Layer 4 — Decision Policy and Trade Opportunity Auction

### 14.1 Purpose
Choose only the best available opportunities.

### 14.2 Candidate Generation Rules
A candidate may be generated only if:
- setup is valid
- no hard override blocks it
- baseline confidence exceeds minimum threshold

### 14.3 Candidate Scoring Dimensions
At minimum:
- asymmetry
- state alignment
- signal confidence
- trigger strength
- trigger confidence
- execution confidence
- OI structure
- liquidation opportunity
- overlap penalties

### 14.4 Auction Rules
- global ranking across candidates
- execute only top `N` or top-notional candidates
- apply diversification and anti-clustering constraints

### 14.5 Diversification Constraint
The auction must penalize:
- highly correlated instruments
- repeated expression of the same thesis
- clustered liquidation exposure
- concentrated funding/fragility expression

### 14.6 Edge Budgeting
Edge budgeting must be treated as a practical throttle using proxies:
- concentration
- overlap
- heat
- exposure
- confidence-adjusted notional

It is **not** an oracle.

---

## 15. Layer 5 — Risk & Portfolio Constraint Engine

### 15.1 Position Sizing Inputs
Sizing must depend on:
- asymmetry
- confidence
- trigger confidence
- degradation level
- execution confidence
- heat
- book concentration

### 15.2 Position Inertia
A max position delta per interval and/or penalty term must prevent excessive flipping.

### 15.3 Quantile-Asymmetry Boost
A modest size increase is allowed only when:
- right tail is strongly favorable
- trigger is confirmed
- execution confidence is acceptable
- reflexivity/heat are not extreme

Hard cap:
- roughly `1.20×` baseline

### 15.4 Prohibited Sizing
- Kelly variants
- streak boosts
- fast performance-chasing leverage changes

### 15.5 Liquidation Exploitation Mode
Allowed only when:
- structure aligns
- trigger is real
- risk bounds hold
- execution confidence is not collapsed

Liquidation is both:
- a risk source
- an offensive opportunity

But always probabilistic and bounded.

---

## 16. Execution Awareness Contract

### 16.1 Principle
Execution is adversarial.

### 16.2 Required Execution Inputs
- spread
- depth
- recent slippage
- venue quality
- fill probability estimate

### 16.3 Execution Confidence
Execution confidence must affect:
- candidate rank
- position size
- urgency
- entry style

### 16.4 Stress Execution Posture
In cascades / dislocations:
- assume worse-than-normal fills
- reduce aggressiveness
- allow trade abandonment if entry quality collapses

### 16.5 Partial Fill Handling
The service must support:
- expected vs actual position reconciliation
- immediate confidence adjustment after partials/failures

### 16.6 Execution Feedback Loop
Execution outcomes must feed into:
- trigger trust
- decision confidence
- future execution style choice

---

## 17. Memory, Feedback, and Slow Adaptation

### 17.1 False Positive Memory
Track recently failed setup classes and temporarily downweight similar patterns.

### 17.2 Opportunity Cost Tracking
Track missed high-quality setups only for slow review and calibration.
Never use it to justify chasing.

### 17.3 Regime Transition Guard
During transitions:
- require stronger confirmation
- reduce initial size
- favor safety over aggression
- but do not hard disable opportunity

### 17.4 Drift Detection
Support lightweight monitoring of:
- feature distribution drift
- output drift
- realized-vs-theoretical edge divergence

### 17.5 Outcome Calibration
Allowed only slowly and weakly, never as a primary driver.

### 17.6 No Fast Self-Rewriting
The service must not perform:
- per-trade policy updates
- rapid weight rewrites
- self-modification based on tiny recent samples

---

## 18. Degradation Hierarchy and Hard Overrides

### 18.1 Degradation States

#### Normal
Full behavior allowed.

#### Reduced
- smaller size
- higher confidence threshold
- fewer opportunities survive auction

#### Defensive
- much smaller size
- fewer aggressive entries
- preference for defense / reduction

#### No-Trade
- no new directional intents
- only reductions / safe neutral behaviors if applicable externally

### 18.2 Hard Override Conditions
Allowed for:
- extreme novelty
- critical exchange failure
- critical liquidity collapse
- corrupted core inputs

### 18.3 Soft Degradation Conditions
Used for:
- elevated heat
- stale structural signals
- elevated execution stress
- transition uncertainty

---

## 19. Configuration Domains

The service must support configuration in these domains:

### Signal Confidence / Decay
- per-signal λ
- confidence floors/caps
- freshness tolerances

### State / Safety
- novelty thresholds
- heat thresholds
- degradation thresholds
- liquidity/spread thresholds
- weekend throttle settings

### Trigger
- setup thresholds
- pre-trigger thresholds
- confirmed trigger thresholds
- missed-move rules

### Auction
- top-N limits
- diversification penalties
- overlap penalties
- minimum auction score

### Risk
- size caps
- asymmetry boost cap
- inertia limits
- concentration limits

### Execution Awareness
- execution confidence thresholds
- stress posture thresholds
- slippage tolerance tiers

### Memory / Adaptation
- false positive memory decay
- transition guard settings
- drift sensitivity
- opportunity cost review windows

---

## 20. Validation and Failure Handling Rules

### 20.1 Validation Classes
Every validation outcome must be classified as:
- `hard_invalid`
- `soft_degraded`
- `recoverable_missing`

### 20.2 Missing Core Data
If missing:
- price
- bid/ask
- spread
- novelty/safety snapshot

Then:
- degrade aggressively or enter no-trade

### 20.3 Missing Non-Core Data
If missing:
- options context
- stablecoin proxy
- local extrema
- some structural auxiliaries

Then:
- continue with lower confidence

### 20.4 Conflicting Signals
If signals conflict:
- reduce confidence
- increase overlap penalty
- demand stronger trigger confirmation

### 20.5 Execution Collapse
If execution quality collapses:
- lower execution confidence
- shrink size
- possibly suppress otherwise-valid opportunities

### 20.6 Regime Uncertainty Spike
If transition probability spikes:
- activate transition guard
- require stronger confirmation
- reduce size

---

## 21. Observability, Logging, and Auditability

The service must expose structured outputs for:
- state transitions
- degradation changes
- novelty overrides
- trigger stage progression
- candidate creation
- candidate ranking
- auction selection
- suppression reasons
- execution confidence changes
- overlap penalties
- false positive memory changes
- realized-vs-theoretical edge tracking

Every material decision must be:
- logged
- attributable
- replayable
- tied to config version and input snapshots

---

## 22. Testing Requirements

### 22.1 Unit Tests
Must cover:
- signal decay
- confidence composition
- OI classification
- trigger stage transitions
- auction ranking
- diversification penalties
- sizing caps
- degradation transitions

### 22.2 Scenario Tests
Must cover:
- liquidation cascade crash
- funding squeeze melt-up
- low-vol chop
- false regime transition
- stale liquidation data
- venue outage
- partial fill stress
- weekend liquidity cliff

### 22.3 Replay Tests
Historical event replays with deterministic reconstruction.

### 22.4 Adversarial Tests
- contradictory signals
- stale data
- corrupted confidence
- delayed feeds
- concentrated auction outcomes
- bad execution environments

### 22.5 Acceptance Tests
The service is acceptable when:
- replayability works
- trigger logic is stable
- diversification prevents hidden clustering
- execution confidence changes behavior in meaningful ways
- stale signals degrade behavior instead of causing chaos
- the service does not depend on one exotic feed to function

---

## 23. Implementation Roadmap

### Phase 1 — Foundation
1. data pipeline contracts + validators
2. signal confidence / freshness / decay
3. state & safety engine
4. novelty + degradation hierarchy
5. single-model quantile output + basic decision snapshot
6. basic trigger layer
7. basic auction skeleton

### Phase 2
8. full compact ensemble
9. OI structure engine
10. funding / basis / liquidation integration
11. diversification-aware auction
12. risk caps and inertia

### Phase 3
13. execution awareness + execution feedback loop
14. false positive memory
15. regime transition guard
16. overlap penalties and edge budgeting proxies

### Phase 4
17. optional PPO overlay
18. heavier conditional GEX usage only if reliability proven

---

## 24. Explicit Non-Goals

The Decision Service does **not** try to:
- perfectly predict price
- trade every opportunity
- eliminate uncertainty
- depend on large model stacks
- perform unconstrained online learning
- replace the OMS/EMS
- solve exchange connectivity
- solve full portfolio accounting
- rely on sentiment as a primary edge

---

## 25. Known Hard Risks

These remain even after mitigation:
- imperfect trigger timing
- execution friction
- data imperfection
- regime misclassification
- edge decay

The service’s job is not to eliminate them.  
Its job is to remain profitable despite them.

---

## 26. Final Build Decision

This unified specification is **complete enough to implement** the Decision Service.

The most important next documents are:

1. **Trigger Math / Pseudocode Spec**
2. **Auction Scoring & Constraint Spec**
3. **Execution Logic Contract**
4. **State Snapshot / Regime Logic Spec**

These define the most important remaining implementation details.

---

## 27. Unified Self-Audit Checklist

This section exists specifically to verify that the final document captured the material conclusions from the full debate.

### 27.1 Core Identity
- [x] leverage-flow exploitation, not generic price prediction
- [x] liquidation, OI, funding, perp/spot, cross-exchange are core
- [x] uncertainty is explicit

### 27.2 Architecture
- [x] full pipeline enumerated
- [x] decision-service-only scope
- [x] no assumptions about platform dependencies
- [x] 5 logical layers + isolated carry sleeve maintained as context

### 27.3 Probabilistic Framework
- [x] signal = value × confidence × freshness
- [x] hard decay rule present
- [x] action under uncertainty rule present
- [x] soft data degradation preferred over paralysis

### 27.4 Feature / Data Contracts
- [x] market snapshot contract
- [x] structural signal contract
- [x] safety/regime contract
- [x] execution feedback contract
- [x] decision snapshot contract
- [x] trigger contract
- [x] candidate trade contract
- [x] trade intent contract
- [x] suppression / override contracts
- [x] decision record contract

### 27.5 State and Safety
- [x] probabilistic regime
- [x] novelty/OOD
- [x] heat score
- [x] degradation hierarchy
- [x] weekend throttle
- [x] exchange risk awareness
- [x] transition guard support

### 27.6 Structure / Forecast
- [x] 3 logical model roles
- [x] quantile outputs
- [x] continuation / fragility / asymmetry
- [x] weak outcome calibration only
- [x] OI structure engine explicit
- [x] liquidation data treated probabilistically
- [x] local extrema auxiliary only

### 27.7 Trigger Logic
- [x] multi-stage trigger
- [x] trigger minimalism
- [x] mechanical trigger types
- [x] missed-move acceptance
- [x] bounded trigger scaling

### 27.8 Auction / Decision
- [x] candidate generation rules
- [x] top-N opportunity auction
- [x] diversification constraint
- [x] overlap penalty
- [x] edge budgeting proxies
- [x] trade quality > trade quantity

### 27.9 Risk
- [x] sizing uses asymmetry + confidence + trigger + execution
- [x] position inertia
- [x] quantile asymmetry boost capped
- [x] Kelly prohibited
- [x] streak sizing prohibited
- [x] liquidation offense/defense bounded

### 27.10 Execution
- [x] execution awareness
- [x] dynamic style selection
- [x] stress posture
- [x] partial fill handling
- [x] execution feedback loop
- [x] worse-than-normal fill assumption in stress

### 27.11 Memory / Adaptation
- [x] false positive memory
- [x] opportunity cost tracking
- [x] lightweight drift detection
- [x] no per-trade updates
- [x] no fast self-rewriting logic

### 27.12 Validation / Testing / Ops
- [x] validation classes
- [x] failure handling rules
- [x] observability requirements
- [x] replayability requirements
- [x] scenario tests
- [x] adversarial tests
- [x] acceptance criteria

### 27.13 Explicit Rejections / Non-Goals
- [x] no hierarchical RL in v1
- [x] no dedicated opponent modeling in v1
- [x] no heavy sentiment core
- [x] no over-engineered consensus blocking logic
- [x] no unconstrained online adaptation

### 27.14 Residual Open Areas
- [x] trigger math still to be formalized
- [x] auction scoring still to be formalized
- [x] execution logic contract still to be formalized
- [x] state/regime logic still to be formalized

### 27.15 Final Audit Conclusion
This document appears to capture all material conclusions and corrections from the debate at the system-spec level.  
Any remaining gaps are now **implementation-detail specs**, not missing architectural findings.

---

## 28. Glossary

### Asymmetry
Relative favorability of upside vs downside distribution.

### Fragility
Likelihood that current structure breaks violently due to leverage/forced flow.

### Reflexivity
Crowding / self-reinforcing positioning pressure.

### Crypto Heat Score
Composite stress/crowding metric used for throttling and safety.

### Degradation Hierarchy
Normal / Reduced / Defensive / No-Trade operating states.

### Trigger
Minimal, mechanical confirmation that the move is actually starting.

### Trade Opportunity Auction
Ranking and selection process for candidate trades.

### False Positive Memory
Memory of recently failed setup types used to temporarily downweight similar setups.

### Edge Budgeting
Practical proxy-based throttle on deployed risk/opportunity concentration.

### Missed Move Acceptance
Explicit refusal to chase extended moves after poor entry timing.
