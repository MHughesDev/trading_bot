# APEX Decision Service — Formal Master Code Specification v1.0

**Document Type**: Software Design / Code Specification  
**Scope**: Decision Service only  
**Version**: 1.0  
**Date**: April 2026  
**Status**: Final  
**System Context**: Crypto perpetual futures trading  
**Primary Objective**: Convert market state, probabilistic leverage-flow signals, and trigger confirmations into a small set of high-quality trade intents with bounded risk and execution-aware confidence.

---

## 1. Purpose

This document defines the **formal code specification** for the **APEX Decision Service**.

It is intentionally:
- **product-agnostic**
- **dependency-agnostic**
- **language-agnostic**
- tightly scoped to the **decision-making bounded context**

It does **not** define the full trading platform, broker/exchange adapters, portfolio accounting system, order management system, or training infrastructure except where minimal interfaces are needed by the Decision Service.

---

## 2. Decision Service Boundaries

## 2.1 In Scope
The Decision Service is responsible for:

1. Receiving normalized market/state/feature snapshots
2. Applying signal confidence, freshness, and decay
3. Building current decision state
4. Producing probabilistic structure/forecast outputs
5. Detecting valid multi-stage triggers
6. Generating candidate trade opportunities
7. Ranking opportunities through a constrained auction
8. Applying hard safety/risk/degradation constraints
9. Producing **trade intents**, **trade suppressions**, or **no-trade outputs**
10. Updating internal decision-relevant memory:
   - signal confidence state
   - false-positive memory
   - opportunity cost tracking
   - trigger trust
   - execution feedback effects
   - transition guard state

## 2.2 Out of Scope
The Decision Service is **not** responsible for:

- market data collection transport details
- raw order placement
- exchange connectivity
- portfolio accounting source of truth
- final margin calculation
- backtesting engine
- training pipelines
- feature computation that belongs to upstream services
- permanent risk monitoring outside decision-time context
- operator UI

The Decision Service may consume outputs from those systems through abstract interfaces.

---

## 3. Core Design Philosophy

1. **Leverage-flow exploitation over generic price prediction**
2. **State drives structure; outcomes only calibrate weakly**
3. **All signals are uncertain**
4. **Timing is gated, not assumed**
5. **Trade quality dominates trade quantity**
6. **Execution realism affects decision quality**
7. **Mitigations must improve realized PnL, not just theoretical safety**
8. **Low confidence should usually reduce size, not cause paralysis**
9. **Hard overrides are allowed only for true danger conditions**
10. **The service must be mechanically testable**

---

## 4. Service Responsibilities by Stage

```text
Input Snapshot
→ Validation / Confidence Normalization
→ State Construction
→ Structure / Forecast Evaluation
→ Trigger Evaluation
→ Candidate Generation
→ Candidate Auction
→ Constraint & Risk Application
→ Trade Intent Output
→ Decision Memory Update
```

---

## 5. External Interfaces (Abstract)

The Decision Service depends on the following abstract upstream/downstream interfaces.

## 5.1 Input Interfaces

### 5.1.1 Market Snapshot Provider
Provides the current normalized market snapshot.

Required fields may include:
- symbol / instrument
- timestamp
- last trade price
- bid / ask / spread
- order book imbalance
- volume burst metrics
- volatility metrics
- liquidity metrics

### 5.1.2 Structural Signal Provider
Provides normalized leverage-flow features.

Required fields may include:
- funding rate
- funding velocity
- open interest
- OI delta
- liquidation cluster proximity
- estimated cascade magnitude
- perp / spot basis
- cross-exchange divergence
- options-derived context when available
- feature freshness
- feature reliability

### 5.1.3 Regime / Safety Provider
Provides current high-level state estimates.

Required fields may include:
- regime probabilities
- novelty / OOD score
- degradation level suggestion
- exchange risk flags
- weekend / low-liquidity mode flags
- crypto heat score
- transition confidence

### 5.1.4 Execution Feedback Provider
Provides decision-relevant execution quality signals from previous actions.

Required fields may include:
- expected vs realized slippage
- fill ratio
- fill latency
- rejected/partial status
- venue degradation indicators

## 5.2 Output Interfaces

### 5.2.1 Trade Intent Sink
Consumes generated trade intents.

### 5.2.2 Decision Log Sink
Consumes structured decision records for audit, replay, and analytics.

### 5.2.3 Decision Metrics Sink
Consumes service metrics and health signals.

---

## 6. Canonical Domain Objects

## 6.1 SignalValue
Represents one normalized signal.

```text
SignalValue
- name: string
- raw_value: float
- normalized_value: float
- confidence: float [0,1]
- freshness: float [0,1]
- decayed_confidence: float [0,1]
- source_count: integer
- reliability_score: float [0,1]
- timestamp: datetime
- metadata: map
```

## 6.2 MarketState
Represents the current decision state.

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

## 6.3 ForecastState
Represents compact probabilistic structure output.

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
```

## 6.4 TriggerState
Represents the trigger pipeline outcome.

```text
TriggerState
- setup_valid: bool
- pretrigger_valid: bool
- trigger_valid: bool
- trigger_strength: float [0,1]
- trigger_confidence: float [0,1]
- trigger_type: enum
- missed_move_flag: bool
```

## 6.5 CandidateTrade
Represents one possible trade.

```text
CandidateTrade
- candidate_id: string
- instrument: string
- side: enum(long, short, flat_reduction)
- entry_style: enum(passive, aggressive, staggered)
- thesis_type: enum(trend, squeeze, liquidation_exploitation, mean_reversion_blocked, carry_neutral)
- state_alignment_score: float
- asymmetry_score: float
- confidence_score: float
- trigger_score: float
- OI_structure_class: enum
- liquidation_opportunity_score: float
- diversification_penalty: float
- auction_score: float
- proposed_size_fraction: float
- hard_reject_reasons: list<string>
- soft_penalties: list<string>
```

## 6.6 TradeIntent
Represents the final decision output.

```text
TradeIntent
- intent_id: string
- instrument: string
- side: enum
- urgency: enum(low, medium, high)
- size_fraction: float
- max_slippage_tolerance: float
- preferred_execution_style: enum
- decision_confidence: float
- degradation_level: enum
- reason_codes: list<string>
- created_at: datetime
```

## 6.7 DecisionRecord
Auditable full record of one decision cycle.

---

## 7. Time and Cadence Model

The Decision Service must support distinct internal cadences.

## 7.1 Fast Safety / State Refresh
Recommended cadence:
- every 1–5 seconds or event-driven on material state change

Used for:
- novelty
- degradation transitions
- liquidity/spread collapse
- exchange risk flags

## 7.2 Structure / Forecast Refresh
Recommended cadence:
- every 1–5 minutes or equivalent feature bar cadence

Used for:
- quantile outputs
- asymmetry
- OI structure
- funding pressure state

## 7.3 Trigger Evaluation
Recommended cadence:
- every 1–30 seconds depending on market activity

## 7.4 Auction / Decision Emission
Recommended cadence:
- event-driven after trigger validation
- or periodic scans over active instruments

---

## 8. Probabilistic Signal Framework

## 8.1 Signal Rule
Every signal must be represented as:

```text
effective_signal = normalized_value × confidence × freshness
```

Where:
- `normalized_value` is direction/magnitude adjusted to service conventions
- `confidence` reflects reliability / source quality / consistency
- `freshness` reflects age and timeliness

## 8.2 Decay Rule
For decaying signals:

```text
decayed_confidence(t) = base_confidence × exp(-λ × age)
```

Where:
- `λ` is tuned per signal family
- age is time elapsed since signal generation or last confirmation

## 8.3 Signal Confidence Rules
Confidence should generally decrease when:
- source count is low
- source consistency is poor
- latency is high
- exchange risk is elevated
- signal family is known to be noisy in current regime

## 8.4 Action Under Uncertainty Curve
Uncertainty usually reduces size rather than forcing rejection.

Guiding rule:
- **low confidence** → smaller size and/or lower rank
- **very low confidence with hard safety conflict** → reject
- **novelty / dislocation overrides** → may force defensive/no-trade regardless

---

## 9. Layer 1 — State & Safety Specification

## 9.1 Inputs
- regime probabilities
- novelty score
- liquidity and spread metrics
- exchange health flags
- weekend / session context
- confidence-adjusted leverage-flow signals

## 9.2 Outputs
- current degradation level
- crypto heat score
- reflexivity score
- hard override flags
- transition risk score

## 9.3 Degradation Hierarchy
The service must represent one of four states:

### Normal
- full decision capability

### Reduced
- lower size multiplier
- higher confidence threshold
- lower trade frequency

### Defensive
- much smaller size multiplier
- very selective entries
- preference for defense / de-risking

### No-Trade
- no new directional trade intents
- allow only safe reductions / isolated carry output if applicable externally

## 9.4 Hard Overrides
Hard overrides are permitted for:
- novelty / OOD extreme
- exchange failure / severe venue degradation
- extreme liquidity collapse
- critical input corruption

---

## 10. Layer 2 — Forecasting / Structure Specification

## 10.1 Purpose
Produce compact decision-relevant structure rather than unconstrained prediction.

## 10.2 Required Outputs
- quantile distribution
- asymmetry score
- continuation probability
- fragility score
- weak directional bias
- model agreement score
- model overlap/correlation penalty

## 10.3 Ensemble Policy
The service assumes three logical model families:
- trend-sensitive sequential
- range/chop-sensitive tabular
- volatility specialist

Implementation is product-agnostic; exact model families may vary if they satisfy these roles.

## 10.4 Calibration Policy
Outcome-aware calibration is permitted only as a weak effect.

Rules:
- default epsilon range: `0.03–0.05`
- maximum epsilon: `0.08`
- only in stable conditions
- half-life: `12–24 hours`
- reset on novelty event or regime shift

## 10.5 OI Structure Engine
The service must classify OI/price structure into at least these canonical cases:

- `OI up / Price up` → healthier continuation
- `OI up / Price flat` → fragile leverage build-up
- `OI down / Price up` → squeeze potential
- `OI down / Price down` → deleveraging / lower continuation confidence

This classification must directly affect:
- asymmetry
- continuation confidence
- liquidation opportunity score

## 10.6 Liquidation Structure Handling
Liquidation information must be treated as:
- probabilistic
- freshness-sensitive
- confidence-weighted

It must never be treated as exact truth.

---

## 11. Layer 3 — Trigger Specification

## 11.1 Purpose
Solve the timing problem.

## 11.2 Trigger Minimalism Requirement
The trigger layer must remain:
- small
- mechanically defined
- backtestable
- explainable

No vague discretionary pattern collections are allowed.

## 11.3 Stages

### Stage 1 — Setup
Requirements:
- asymmetry above threshold
- state alignment above threshold
- no hard conflict from heat / novelty / degradation

### Stage 2 — Pre-Trigger
Requirements:
- pressure-building evidence, such as:
  - order-flow imbalance shift
  - microstructure tightening
  - early volume expansion

### Stage 3 — Confirmed Trigger
Requirements:
- one or more precise trigger events such as:
  - order book imbalance spike beyond threshold
  - volume burst beyond threshold
  - break of local structure / breakout confirmation

## 11.4 Trigger Output Rules
- Trigger confidence may scale size only within bounded limits
- The service must support **missed move acceptance**
- If the move has clearly run without acceptable entry quality, the candidate must be dropped rather than chased

---

## 12. Layer 4 — Candidate Generation and Trade Opportunity Auction

## 12.1 Candidate Generation
A candidate may be created only if:
- setup is valid
- no hard override blocks it
- baseline confidence exceeds minimum threshold

## 12.2 Candidate Scoring Inputs
Each candidate must be scored using at least:
- asymmetry
- state alignment
- signal confidence
- trigger strength
- trigger confidence
- OI structure classification
- liquidation exploitation score
- execution confidence
- diversification penalty

## 12.3 Auction Rules
- candidates are globally ranked
- only top `N` or top-notional candidates may proceed
- auction must include diversification logic so top candidates do not collapse into one correlated idea

## 12.4 Diversification Constraint
The auction must penalize:
- highly correlated candidates
- repeated expression of same thesis
- concentrated exposure in same structural driver

---

## 13. Layer 5 — Risk and Constraint Specification

## 13.1 Position Sizing
Sizing must depend on:
- asymmetry
- signal confidence
- trigger confidence
- degradation level
- execution confidence
- current book concentration

## 13.2 Position Inertia
The service must enforce a maximum position delta per decision interval and/or penalty term to prevent excessive flipping.

## 13.3 Quantile-Asymmetry Boost
The service may modestly increase size only when:
- right tail is strongly favorable
- reflexivity / heat are not extreme
- trigger confidence is high
- execution confidence is acceptable

Hard cap:
- approximately `1.20×` baseline size

## 13.4 Prohibited Sizing Methods
- Kelly sizing
- raw fractional Kelly
- streak-based boost
- fast outcome-driven leverage increase

## 13.5 Edge Budgeting
The service may track practical deployed-edge proxies:
- concentration
- heat
- overlap
- exposure
- confidence-adjusted notional

This is a throttle, not an exact oracle.

---

## 14. Execution Awareness Specification

## 14.1 Principle
Execution is adversarial and must be assumed imperfect.

## 14.2 Required Inputs
- spread
- depth
- recent slippage
- venue quality
- fill probability estimate

## 14.3 Execution Confidence
Execution confidence must affect:
- candidate ranking
- size
- urgency
- preferred execution style

## 14.4 Stress Posture
In cascades or severe dislocations:
- assume worse-than-normal fills
- prefer safer behavior over theoretical best entry
- allow the trade thesis to be abandoned if execution quality collapses

## 14.5 Fill Reconciliation Hook
The service must be able to incorporate actual fill outcomes into subsequent decision confidence adjustments.

---

## 15. Long-Horizon Memory and Adaptation

## 15.1 False Positive Memory
The service should track recently failed setup types and temporarily downweight similar setups.

## 15.2 Opportunity Cost Tracking
Track missed high-quality opportunities only for slow calibration and review, never as a real-time chase mechanism.

## 15.3 Regime Transition Guard
During regime transitions:
- require higher confidence
- reduce initial size
- favor confirmation over aggression

## 15.4 Drift Detection
The service should support lightweight detection of:
- feature distribution drift
- output drift
- realized-vs-theoretical edge divergence

## 15.5 Shadow Logic Compatibility
The service should be compatible with shadow/scored alternative logic without changing live behavior until promoted externally.

---

## 16. Decision Output Semantics

The service may emit one of the following:

1. **TradeIntent**
2. **SuppressIntent**
   - valid setup existed but was intentionally blocked
3. **NoTrade**
4. **ReduceExposureIntent**
5. **ConfidenceDowngradeEvent**
6. **SafetyOverrideEvent**

Each output must contain:
- reason codes
- confidence
- timestamps
- degradation context
- major contributing scores

---

## 17. Observability and Audit Requirements

The service must expose structured data for:

- state transitions
- trigger activation
- candidate ranking
- auction results
- rejection reasons
- hard override reasons
- execution confidence changes
- false positive memory updates
- realized-vs-theoretical edge tracking

Every material decision must be replayable from logged inputs and config versions.

---

## 18. Configuration Domains

The service must support configuration at least across these domains:

### Signal Confidence / Decay
- per-signal λ
- confidence floors/caps

### State / Safety
- novelty thresholds
- heat thresholds
- degradation thresholds
- liquidity/spread thresholds

### Trigger
- setup thresholds
- pre-trigger thresholds
- trigger event thresholds
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

### Adaptation / Memory
- false-positive memory decay
- regime transition guard thresholds
- drift sensitivity

---

## 19. Failure Handling Rules

## 19.1 Missing / Degraded Inputs
- lower confidence
- degrade size
- escalate degradation level if severe
- do not hard-fail unless core safety inputs are unavailable

## 19.2 Conflicting Signals
- reduce confidence
- increase overlap penalty
- possibly suppress action if trigger quality is insufficient

## 19.3 Execution Collapse
- lower execution confidence
- shrink size
- prefer no-trade or defensive behavior

## 19.4 Regime Uncertainty Spike
- activate transition guard
- require stronger confirmation

---

## 20. Test Requirements

The Decision Service must be testable at these levels.

## 20.1 Unit Tests
- signal decay
- confidence computation
- OI structure classification
- trigger stage transitions
- auction ranking
- diversification penalty
- sizing caps
- degradation transitions

## 20.2 Scenario Tests
- cascade crash
- funding squeeze melt-up
- low-vol chop
- false regime transition
- stale liquidation data
- venue outage / fragmentation
- partial fill stress

## 20.3 Replay Tests
- historical event replays with fixed config
- deterministic reconstruction of decision outputs

## 20.4 Adversarial Tests
- contradictory signals
- corrupted confidence
- delayed feeds
- unrealistic but dangerous concentration cases

---

## 21. Acceptance Criteria for Production Readiness

The Decision Service is production-ready when:

1. all core decisions are replayable
2. trigger logic is explicit and stable
3. auction diversification prevents hidden clustering
4. execution confidence demonstrably influences decision quality
5. realized-vs-theoretical edge can be monitored
6. stale/imperfect signals degrade behavior rather than cause chaos
7. no hard dependency on any one exotic signal source exists

---

## 22. Explicit Non-Goals

This service does not attempt to:
- perfectly predict price
- eliminate all uncertainty
- chase every move
- optimize for maximum trade count
- replace all other platform services
- solve exchange connectivity
- run unconstrained online learning

---

## 23. Final Build Decision

This Decision Service specification is **complete enough to implement**.

The best next deliverable is one of:
1. **Feature schema & data contracts**
2. **Trigger math / pseudocode**
3. **Execution logic contract**
4. **State snapshot schema**
