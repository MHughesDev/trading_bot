# APEX — State / Regime Logic Detail Spec v1.0

**Document Type**: Implementation Detail Specification  
**Scope**: State construction, regime inference, degradation logic, heat/reflexivity, and transition handling  
**Version**: 1.0  
**Date**: April 2026  
**Status**: Build-ready draft  
**Parent Spec**: APEX Unified Full-System Master Spec v2.0

---

## 1. Purpose

This document formalizes the **state and regime logic** for APEX.

The state engine is the system’s world model.  
It determines:

- what kind of market is currently present
- how confident that classification is
- how stressed or fragile the market is
- whether the system should trade normally, reduce, defend, or stop
- how much uncertainty should propagate downstream

---

## 2. Design Principles

1. Regime is probabilistic, not binary
2. Transition risk matters as much as steady-state classification
3. Heat and novelty can override otherwise attractive structure
4. Low confidence should usually reduce aggression, not create unexplained behavior
5. State must be compact, explainable, and replayable

---

## 3. Inputs

The state engine consumes:

### 3.1 Market Inputs
- short and medium realized volatility
- spread state
- liquidity/depth state
- order book imbalance stability
- volume regime
- session / weekend context

### 3.2 Structural Inputs
- funding z-score and velocity
- OI level / delta / concentration
- basis / perp-spot divergence
- liquidation proximity and density
- cross-exchange divergence

### 3.3 Safety Inputs
- novelty/OOD score
- exchange risk flags
- data integrity warnings
- execution stress signals

---

## 4. Regime Classes

Minimum required logical classes:
- `trend`
- `range`
- `stress`
- `dislocated`
- `transition`

### 4.1 Meanings
- `trend`: directional continuation likely
- `range`: chop / low directional edge / mean-reverting conditions
- `stress`: high volatility, low stability, higher forced-flow risk
- `dislocated`: abnormal market structure, fragmentation, outages, or severe inconsistency
- `transition`: unstable classification or shifting regime boundaries

---

## 5. Regime Probability Vector

Let:

```text
R = {p_trend, p_range, p_stress, p_dislocated, p_transition}
```

Rules:
- probabilities must sum approximately to 1
- no hard regime labels should be used as sole truth
- downstream logic should read both probabilities and confidence

---

## 6. Regime Confidence

### 6.1 Basic Form
One acceptable confidence measure:

```text
regime_confidence = max(R) - second_max(R)
```

Alternative bounded separation metrics are allowed if:
- deterministic
- replayable
- interpretable

### 6.2 Interpretation
- high → regime class is relatively clear
- low → transition risk or ambiguity is elevated

---

## 7. Transition Probability

### 7.1 Purpose
Detect when market structure is shifting so aggression should be moderated.

### 7.2 Inputs
- falling regime confidence
- rising volatility instability
- disagreement between structural and microstructure conditions
- abrupt feature direction changes
- rapid changes in heat/reflexivity/liquidity

### 7.3 Example Formula

Let:
- `Rc` = inverse regime confidence
- `Vt` = volatility transition score
- `Md` = microstructure disagreement score
- `Sd` = structural disagreement score

```text
transition_probability_raw = t1*Rc + t2*Vt + t3*Md + t4*Sd
transition_probability = clip(transition_probability_raw, 0, 1)
```

---

## 8. Crypto Heat Score

### 8.1 Purpose
Provide a global stress/crowding metric that throttles aggression.

### 8.2 Components

Let:
- `Hf` = funding extremity
- `Hl` = liquidation pressure / proximity
- `Ho` = OI fragility / concentration
- `Hx` = cross-exchange divergence
- `Hv` = volatility stress
- `He` = execution stress

### 8.3 Formula

```text
heat_score_raw = b1*Hf + b2*Hl + b3*Ho + b4*Hx + b5*Hv + b6*He
heat_score = clip(heat_score_raw, 0, 1)
```

### 8.4 Requirements
- coefficients must be configurable
- heat score must be logged
- heat score must be explainable by component breakdown

---

## 9. Reflexivity Score

### 9.1 Purpose
Measure crowding/self-reinforcing positioning pressure.

### 9.2 Inputs
- funding z-score
- OI concentration
- basis distortion
- options skew when reliable
- crowded positioning proxies

### 9.3 Role
Reflexivity should primarily:
- cap size
- tighten aggression
- lower confidence at extremes

It must **not** be used as a naive directional flip by itself.

---

## 10. Novelty / OOD

### 10.1 Purpose
Detect when the system is outside familiar structure.

### 10.2 Behavior
Novelty may:
- raise degradation level
- block new directional trades in extreme conditions
- increase trigger thresholds
- reduce confidence across many feature families

### 10.3 Logging
Novelty must always emit:
- score
- threshold status
- reason code(s)

---

## 11. Degradation Hierarchy

### 11.1 States

#### Normal
- full behavior allowed

#### Reduced
- smaller size multiplier
- higher confidence thresholds
- lower candidate count

#### Defensive
- much smaller size
- very selective new trades
- preference for defensive reductions

#### No-Trade
- no new directional intents
- allow only safe reductions / neutral behaviors if configured

### 11.2 Transition Logic

Example:

```text
if hard_override:
    degradation = no_trade
elif novelty_score >= novelty_critical or exchange_risk == critical:
    degradation = no_trade
elif heat_score >= heat_defensive_threshold or execution_stress_high:
    degradation = defensive
elif heat_score >= heat_reduced_threshold or transition_probability >= transition_threshold:
    degradation = reduced
else:
    degradation = normal
```

---

## 12. Weekend / Low-Liquidity Mode

### 12.1 Purpose
Adjust aggression for thin books and liquidity cliffs.

### 12.2 Behavior
Weekend / low-liquidity mode should:
- reduce aggression
- raise trigger quality requirements
- increase execution caution

It should not:
- automatically shut down all trading
- assume no good trades exist

---

## 13. Exchange Risk Awareness

### 13.1 Inputs
- abnormal spreads
- API degradation
- venue quality deterioration
- stale data conditions
- outage flags

### 13.2 Effects
Exchange risk may:
- lower confidence
- raise degradation
- suppress venue-dependent trades
- increase execution stress components

---

## 14. Confidence Vector

The state engine must produce a confidence vector covering at least:
- market confidence
- structural confidence
- triggerable confidence
- execution confidence context
- regime confidence
- data integrity confidence

Downstream layers use this vector instead of hidden assumptions.

---

## 15. State Engine Pseudocode

```text
function build_state(market_snapshot, structural_snapshot, safety_snapshot, execution_feedback):
    regime_probs = normalize_regime_probs(safety_snapshot.regime_probabilities)
    regime_conf = compute_regime_confidence(regime_probs)

    transition_prob = compute_transition_probability(
        market_snapshot,
        structural_snapshot,
        regime_probs
    )

    heat_score = compute_heat(
        structural_snapshot,
        market_snapshot,
        execution_feedback
    )

    reflexivity_score = compute_reflexivity(structural_snapshot)

    degradation = compute_degradation(
        novelty=safety_snapshot.novelty_score,
        exchange_risk=safety_snapshot.exchange_risk_level,
        heat=heat_score,
        transition_probability=transition_prob,
        execution_feedback=execution_feedback
    )

    confidence_vector = build_confidence_vector(
        market_snapshot,
        structural_snapshot,
        safety_snapshot,
        execution_feedback
    )

    return MarketState(
        timestamp=market_snapshot.timestamp,
        regime_probabilities=regime_probs,
        regime_confidence=regime_conf,
        transition_probability=transition_prob,
        novelty_score=safety_snapshot.novelty_score,
        volatility_state=classify_volatility(market_snapshot),
        liquidity_state=classify_liquidity(market_snapshot),
        spread_state=classify_spread(market_snapshot),
        weekend_mode=safety_snapshot.weekend_mode,
        exchange_risk_level=safety_snapshot.exchange_risk_level,
        degradation_level=degradation,
        crypto_heat_score=heat_score,
        reflexivity_score=reflexivity_score,
        confidence_vector=confidence_vector
    )
```

---

## 16. Test Requirements

### Unit Tests
- regime probability normalization
- regime confidence calculation
- transition probability behavior
- heat score component composition
- degradation transitions
- weekend throttle effects
- exchange risk effects

### Scenario Tests
- clean trend
- low-vol chop
- funding squeeze
- liquidation cascade
- false regime flip
- stale structural data
- venue outage / fragmented market

### Replay Tests
- real crash sequences
- real melt-up sequences
- weekend liquidity cliffs

---

## 17. Acceptance Criteria

The state engine is acceptable when:
1. it produces coherent and explainable state vectors
2. transition risk moderates aggression during flips
3. heat score materially affects behavior without freezing the system
4. novelty and exchange risk can trigger hard protective states
5. all state transitions are replayable and logged
